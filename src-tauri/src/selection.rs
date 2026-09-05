use crate::{
    contracts::{
        DEFAULT_SELECTION_MODE, DEFAULT_SELECTION_SHORTCUT, DEFAULT_SELECTION_WINDOW_HEIGHT,
        DEFAULT_SELECTION_WINDOW_WIDTH, MAX_SELECTION_WINDOW_HEIGHT, MAX_SELECTION_WINDOW_WIDTH,
        MIN_SELECTION_WINDOW_HEIGHT, MIN_SELECTION_WINDOW_WIDTH, SelectionAnchor, SelectionMode,
        SelectionNotice, SelectionRequestPayload, SelectionRuntimeStatus, SelectionStatusChanged,
        SelectionTrigger, SelectionTriggerNotice, SelectionUnavailable,
    },
    diagnostics,
};
use std::{
    sync::{Arc, Mutex, mpsc},
    thread,
    time::{Duration, Instant},
};
use tauri::window::Color;
use tauri::{AppHandle, Emitter, LogicalSize, Manager, WebviewUrl, WebviewWindowBuilder};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

const SELECTION_EVENT: &str = "selection_available";
const SELECTION_TRIGGER_EVENT: &str = "selection_trigger_available";
const SELECTION_UNAVAILABLE_EVENT: &str = "selection_unavailable";
const SELECTION_STATUS_EVENT: &str = "selection_status_changed";
const SELECTION_OPEN_MAIN_EVENT: &str = "selection_open_main";
const SELECTION_TTL: Duration = Duration::from_secs(120);
const AUTOMATIC_DEBOUNCE: Duration = Duration::from_millis(500);
const FOCUS_POLL_INTERVAL: Duration = Duration::from_millis(120);
const MOUSE_RELEASE_DELAY: Duration = Duration::from_millis(140);
const CLIPBOARD_CAPTURE_TIMEOUT: Duration = Duration::from_millis(900);
const CLIPBOARD_POLL_INTERVAL: Duration = Duration::from_millis(20);
const SHORTCUT_RELEASE_TIMEOUT: Duration = Duration::from_millis(700);
const MAX_SELECTION_ANCESTORS: usize = 8;
const DRAG_FOCUS_GRACE: Duration = Duration::from_millis(250);
const RESIZE_FOCUS_GRACE: Duration = Duration::from_millis(750);
const FOCUS_LOST_CHECK_DELAY: Duration = Duration::from_millis(320);

#[derive(Clone)]
pub struct SelectionService {
    inner: Arc<Mutex<SelectionInner>>,
    worker: Arc<Mutex<Option<mpsc::Sender<WorkerCommand>>>>,
    app: Arc<Mutex<Option<AppHandle>>>,
    cancellations: Arc<Mutex<std::collections::HashMap<String, CancellationToken>>>,
}

struct SelectionInner {
    mode: SelectionMode,
    shortcut: String,
    shortcut_registered: bool,
    ui_automation_ready: bool,
    message: Option<String>,
    source_language: String,
    target_language: String,
    selection_generation: u64,
    latest: Option<StoredSelection>,
    current_trigger: Option<SelectionTriggerNotice>,
    pending_trigger: Option<SelectionTriggerNotice>,
    pending_event: Option<PendingSelectionEvent>,
    window_ready: bool,
    last_signature: Option<SelectionSignature>,
    dragging: bool,
    drag_finished_at: Option<Instant>,
    last_resized_at: Option<Instant>,
    focus_lost_generation: u64,
    content_width: i64,
    content_height: i64,
}

enum PendingSelectionEvent {
    Available(SelectionNotice),
    Unavailable(SelectionUnavailable),
}

struct StoredSelection {
    payload: SelectionRequestPayload,
    created_at: Instant,
}

#[derive(Clone, PartialEq)]
struct SelectionSignature {
    source_text: String,
    anchor: Option<SelectionAnchor>,
}

#[derive(Clone)]
struct SelectionCandidate {
    trigger_id: String,
    source_text: String,
    anchor: Option<SelectionAnchor>,
    trigger: SelectionTrigger,
}

#[cfg(windows)]
type AgileTextRange =
    windows::core::AgileReference<windows::Win32::UI::Accessibility::IUIAutomationTextRange>;

#[cfg(windows)]
#[derive(Clone)]
struct PreparedSelection {
    trigger_id: String,
    generation: u64,
    trigger: SelectionTrigger,
    anchor: Option<SelectionAnchor>,
    source: PreparedSelectionSource,
}

#[cfg(windows)]
#[derive(Clone)]
enum PreparedSelectionSource {
    UiAutomation(Vec<AgileTextRange>),
    Clipboard(String),
}

#[cfg(windows)]
struct MouseSelectionOperation {
    process_id: Option<u32>,
}

#[cfg(windows)]
struct MouseSelectionRelease {
    process_id: Option<u32>,
    anchor: Option<SelectionAnchor>,
    generation: u64,
    deadline: Instant,
}

#[cfg(windows)]
struct SelectionCaptureFailure {
    code: &'static str,
    message: String,
}

#[cfg(windows)]
impl SelectionCaptureFailure {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

enum WorkerCommand {
    SetAutomatic(bool),
    ReadFocused {
        trigger_id: String,
    },
    ActivateTrigger {
        trigger_id: String,
    },
    ClearPending,
    #[cfg(windows)]
    Prepared(PreparedSelection),
    Shutdown,
}

impl SelectionService {
    pub fn new(
        cancellations: Arc<Mutex<std::collections::HashMap<String, CancellationToken>>>,
    ) -> Self {
        Self {
            inner: Arc::new(Mutex::new(SelectionInner {
                mode: DEFAULT_SELECTION_MODE,
                shortcut: DEFAULT_SELECTION_SHORTCUT.to_string(),
                shortcut_registered: false,
                ui_automation_ready: false,
                message: None,
                source_language: "en".to_string(),
                target_language: "zh-CN".to_string(),
                selection_generation: 0,
                latest: None,
                current_trigger: None,
                pending_trigger: None,
                pending_event: None,
                window_ready: false,
                last_signature: None,
                dragging: false,
                drag_finished_at: None,
                last_resized_at: None,
                focus_lost_generation: 0,
                content_width: DEFAULT_SELECTION_WINDOW_WIDTH,
                content_height: DEFAULT_SELECTION_WINDOW_HEIGHT,
            })),
            worker: Arc::new(Mutex::new(None)),
            app: Arc::new(Mutex::new(None)),
            cancellations,
        }
    }

    pub fn attach_app(&self, app: AppHandle) {
        if let Ok(mut stored) = self.app.lock() {
            *stored = Some(app);
        }
    }

    pub fn start_worker(&self) {
        let mut worker_guard = match self.worker.lock() {
            Ok(value) => value,
            Err(_) => return,
        };
        if worker_guard.is_some() {
            return;
        }
        let (sender, receiver) = mpsc::channel();
        let service = self.clone();
        let result = thread::Builder::new()
            .name("lilt-selection-worker".to_string())
            .spawn(move || worker_loop(receiver, service));
        match result {
            Ok(_) => {
                diagnostics::info("selection.worker.start");
                *worker_guard = Some(sender);
            }
            Err(error) => diagnostics::error(format!("selection.worker.failed reason={error}")),
        }
    }

    pub fn status(&self) -> SelectionRuntimeStatus {
        let inner = self.inner.lock().expect("selection state lock poisoned");
        SelectionRuntimeStatus {
            mode: inner.mode,
            shortcut: inner.shortcut.clone(),
            shortcut_registered: inner.shortcut_registered,
            ui_automation_ready: inner.ui_automation_ready,
            message: inner.message.clone(),
        }
    }

    pub fn set_language(&self, source_language: String, target_language: String) {
        if let Ok(mut inner) = self.inner.lock() {
            if !source_language.trim().is_empty() {
                inner.source_language = source_language.trim().to_string();
            }
            if !target_language.trim().is_empty() {
                inner.target_language = target_language.trim().to_string();
            }
        }
    }

    pub fn set_window_size(&self, width: i64, height: i64) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.content_width =
                width.clamp(MIN_SELECTION_WINDOW_WIDTH, MAX_SELECTION_WINDOW_WIDTH);
            inner.content_height =
                height.clamp(MIN_SELECTION_WINDOW_HEIGHT, MAX_SELECTION_WINDOW_HEIGHT);
        }
    }

    fn window_size(&self) -> (i64, i64) {
        self.inner
            .lock()
            .map(|inner| (inner.content_width, inner.content_height))
            .unwrap_or((
                DEFAULT_SELECTION_WINDOW_WIDTH,
                DEFAULT_SELECTION_WINDOW_HEIGHT,
            ))
    }

    pub fn configure(
        &self,
        app: &AppHandle,
        mode: SelectionMode,
        shortcut: &str,
    ) -> Result<SelectionRuntimeStatus, String> {
        let shortcut = shortcut.trim();
        if shortcut.is_empty() {
            return Err("全局快捷键不能为空".to_string());
        }
        let normalized = normalize_shortcut(shortcut);
        #[cfg(desktop)]
        let parsed = normalized
            .parse::<tauri_plugin_global_shortcut::Shortcut>()
            .map_err(|error| format!("快捷键格式无效：{error}"))?;

        let previous = self.status();
        #[cfg(desktop)]
        {
            use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};
            let shortcuts = app.global_shortcut();
            shortcuts
                .unregister_all()
                .map_err(|error| format!("清理旧快捷键失败：{error}"))?;
            if mode == SelectionMode::Shortcut {
                let service = self.clone();
                if let Err(error) = shortcuts.on_shortcut(parsed, move |_app, _shortcut, event| {
                    if event.state == ShortcutState::Pressed {
                        service.read_focused_selection();
                    }
                }) {
                    if previous.mode == SelectionMode::Shortcut {
                        let old = normalize_shortcut(&previous.shortcut);
                        if let Ok(old_shortcut) =
                            old.parse::<tauri_plugin_global_shortcut::Shortcut>()
                        {
                            let service = self.clone();
                            let _ = shortcuts.on_shortcut(
                                old_shortcut,
                                move |_app, _shortcut, event| {
                                    if event.state == ShortcutState::Pressed {
                                        service.read_focused_selection();
                                    }
                                },
                            );
                        }
                    }
                    if let Ok(mut inner) = self.inner.lock() {
                        inner.shortcut_registered = previous.shortcut_registered;
                        inner.message = Some(format!("快捷键注册失败：{error}"));
                    }
                    self.emit_status();
                    return Err(format!("快捷键注册失败：{error}"));
                }
            }
        }

        if let Ok(mut inner) = self.inner.lock() {
            inner.mode = mode;
            inner.shortcut = shortcut.to_string();
            inner.shortcut_registered = mode == SelectionMode::Shortcut;
            inner.message = None;
        }
        self.send_worker(WorkerCommand::SetAutomatic(
            mode == SelectionMode::Automatic,
        ));
        self.emit_status();
        Ok(self.status())
    }

    pub fn read_focused_selection(&self) {
        let trigger_id = Uuid::new_v4().to_string();
        self.invalidate_for_new_selection();
        self.publish_trigger(trigger_id.clone(), SelectionTrigger::Shortcut, None);
        if !self.send_worker(WorkerCommand::ReadFocused {
            trigger_id: trigger_id.clone(),
        }) {
            self.publish_unavailable(
                &trigger_id,
                SelectionTrigger::Shortcut,
                "selection_worker_unavailable",
                "划词工作线程不可用",
            );
        }
    }

    pub fn window_ready(&self) -> Option<SelectionTriggerNotice> {
        let (pending, pending_event) = {
            let mut inner = self.inner.lock().ok()?;
            inner.window_ready = true;
            (inner.pending_trigger.take(), inner.pending_event.take())
        };
        if let Some(notice) = pending.as_ref() {
            if let Some(app) = self.app_handle() {
                let _ = app.emit(SELECTION_TRIGGER_EVENT, notice);
            }
            self.show_for_trigger(notice);
        }
        if let Some(event) = pending_event {
            match event {
                PendingSelectionEvent::Available(notice) => {
                    if let Some(app) = self.app_handle() {
                        let _ = app.emit(SELECTION_EVENT, &notice);
                    }
                    self.show_window(notice.anchor.as_ref());
                }
                PendingSelectionEvent::Unavailable(notice) => {
                    if let Some(app) = self.app_handle() {
                        let _ = app.emit(SELECTION_UNAVAILABLE_EVENT, &notice);
                    }
                    if notice.trigger == SelectionTrigger::Shortcut {
                        self.show_window(None);
                    }
                }
            }
        }
        pending
    }

    pub fn activate_trigger(&self, trigger_id: &str) -> Result<(), String> {
        let valid = self
            .inner
            .lock()
            .map_err(|_| "选区状态锁已损坏".to_string())?
            .current_trigger
            .as_ref()
            .map(|notice| notice.trigger_id == trigger_id)
            .unwrap_or(false);
        if !valid {
            return Err("划词触发已过期".to_string());
        }
        let anchor = self.inner.lock().ok().and_then(|inner| {
            inner
                .current_trigger
                .as_ref()
                .and_then(|value| value.anchor.clone())
        });
        self.show_window(anchor.as_ref());
        if !self.send_worker(WorkerCommand::ActivateTrigger {
            trigger_id: trigger_id.to_string(),
        }) {
            return Err("划词工作线程不可用".to_string());
        }
        diagnostics::info(format!(
            "selection.trigger.activate trigger_id={trigger_id}"
        ));
        Ok(())
    }

    fn invalidate_for_new_selection(&self) {
        let (active_id, should_hide) = {
            let mut inner = match self.inner.lock() {
                Ok(value) => value,
                Err(_) => return,
            };
            inner.selection_generation = inner.selection_generation.wrapping_add(1);
            let active_id = inner.latest.take().map(|item| item.payload.request_id);
            let should_hide = inner.current_trigger.take().is_some() || active_id.is_some();
            inner.pending_trigger = None;
            inner.pending_event = None;
            inner.last_signature = None;
            (active_id, should_hide)
        };
        self.cancel_request(active_id);
        if should_hide {
            self.hide_window();
        }
    }

    pub fn get_request(&self, request_id: &str) -> Result<SelectionRequestPayload, String> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| "选区状态锁已损坏".to_string())?;
        let selection = inner
            .latest
            .as_ref()
            .filter(|value| value.payload.request_id == request_id)
            .ok_or_else(|| "选区请求已过期".to_string())?;
        if selection.created_at.elapsed() > SELECTION_TTL {
            inner.latest = None;
            return Err("选区请求已过期".to_string());
        }
        Ok(selection.payload.clone())
    }

    pub fn open_in_main(&self, request_id: &str) -> Result<(), String> {
        let _ = self.get_request(request_id)?;
        let app = self
            .app_handle()
            .ok_or_else(|| "应用尚未完成初始化".to_string())?;
        app.emit(SELECTION_OPEN_MAIN_EVENT, request_id.to_string())
            .map_err(|error| format!("打开主窗口失败：{error}"))?;
        if let Some(window) = app.get_webview_window("selection") {
            let _ = window.hide();
        }
        if let Some(window) = app.get_webview_window("main") {
            let _ = window.show();
            let _ = window.set_focus();
        }
        Ok(())
    }

    pub fn dismiss(&self, request_id: Option<&str>) {
        let (should_dismiss, active_id) = {
            let mut inner = match self.inner.lock() {
                Ok(value) => value,
                Err(_) => return,
            };
            let request_matches = request_id.is_none()
                || inner
                    .latest
                    .as_ref()
                    .map(|item| item.payload.request_id.as_str())
                    == request_id;
            if !request_matches {
                (false, None)
            } else {
                if request_id.is_none() {
                    inner.current_trigger = None;
                    inner.pending_trigger = None;
                    inner.pending_event = None;
                    inner.selection_generation = inner.selection_generation.wrapping_add(1);
                }
                (
                    true,
                    inner.latest.take().map(|item| item.payload.request_id),
                )
            }
        };
        if !should_dismiss {
            return;
        }
        self.send_worker(WorkerCommand::ClearPending);
        self.cancel_request(active_id);
        if let Some(app) = self.app_handle() {
            if let Some(window) = app.get_webview_window("selection") {
                let _ = window.hide();
            }
        }
    }

    pub fn hide_window(&self) {
        if let Some(app) = self.app_handle() {
            if let Some(window) = app.get_webview_window("selection") {
                let _ = window.hide();
            }
        }
    }

    pub fn begin_drag(&self) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.dragging = true;
            inner.drag_finished_at = None;
        }
    }

    pub fn end_drag(&self) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.dragging = false;
            inner.drag_finished_at = Some(Instant::now());
        }
    }

    pub fn handle_window_resized(&self) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.last_resized_at = Some(Instant::now());
            inner.focus_lost_generation = inner.focus_lost_generation.wrapping_add(1);
        }
    }

    pub fn handle_focus_gained(&self) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.focus_lost_generation = inner.focus_lost_generation.wrapping_add(1);
        }
    }

    pub fn handle_focus_lost(&self) {
        let generation = match self.inner.lock() {
            Ok(mut inner) => {
                inner.focus_lost_generation = inner.focus_lost_generation.wrapping_add(1);
                inner.focus_lost_generation
            }
            Err(_) => return,
        };
        let service = self.clone();
        thread::spawn(move || {
            thread::sleep(FOCUS_LOST_CHECK_DELAY);
            service.hide_after_focus_lost(generation);
        });
    }

    fn hide_after_focus_lost(&self, generation: u64) {
        let should_hide = self
            .inner
            .lock()
            .map(|inner| {
                inner.focus_lost_generation == generation
                    && !inner.dragging
                    && inner
                        .drag_finished_at
                        .map(|finished_at| finished_at.elapsed() >= DRAG_FOCUS_GRACE)
                        .unwrap_or(true)
                    && !recently_resized(inner.last_resized_at)
                    && inner.current_trigger.is_none()
            })
            .unwrap_or(false);
        if should_hide {
            self.hide_window();
        }
    }

    pub fn shutdown(&self) {
        let sender = self.worker.lock().ok().and_then(|mut worker| worker.take());
        if let Some(sender) = sender {
            let _ = sender.send(WorkerCommand::Shutdown);
        }
    }

    fn send_worker(&self, command: WorkerCommand) -> bool {
        if let Ok(worker) = self.worker.lock() {
            if let Some(sender) = worker.as_ref() {
                return sender.send(command).is_ok();
            }
        }
        false
    }

    fn cancel_request(&self, request_id: Option<String>) {
        if let Some(request_id) = request_id {
            if let Ok(mut cancellations) = self.cancellations.lock() {
                if let Some(token) = cancellations.remove(&request_id) {
                    token.cancel();
                }
            }
        }
    }

    fn app_handle(&self) -> Option<AppHandle> {
        self.app.lock().ok()?.clone()
    }

    fn set_uia_status(&self, ready: bool, message: Option<String>) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.ui_automation_ready = ready;
            inner.message = message;
        }
        diagnostics::info(format!("selection.worker.status ready={ready}"));
        self.emit_status();
    }

    fn emit_status(&self) {
        let status = self.status();
        if let Some(app) = self.app_handle() {
            let _ = app.emit(
                SELECTION_STATUS_EVENT,
                SelectionStatusChanged {
                    mode: status.mode,
                    shortcut: status.shortcut,
                    shortcut_registered: status.shortcut_registered,
                    ui_automation_ready: status.ui_automation_ready,
                    message: status.message,
                },
            );
        }
    }

    fn emit_unavailable(
        &self,
        trigger_id: &str,
        trigger: SelectionTrigger,
        code: &str,
        message: &str,
    ) {
        self.publish_unavailable(trigger_id, trigger, code, message);
    }

    fn publish_unavailable(
        &self,
        trigger_id: &str,
        trigger: SelectionTrigger,
        code: &str,
        message: &str,
    ) {
        self.publish_unavailable_with_options(trigger_id, trigger, code, message, false);
    }

    fn publish_activation_unavailable(
        &self,
        trigger_id: &str,
        trigger: SelectionTrigger,
        code: &str,
        message: &str,
    ) {
        self.publish_unavailable_with_options(trigger_id, trigger, code, message, true);
    }

    fn publish_unavailable_with_options(
        &self,
        trigger_id: &str,
        trigger: SelectionTrigger,
        code: &str,
        message: &str,
        preserve_trigger: bool,
    ) {
        let notice = SelectionUnavailable {
            request_id: None,
            trigger_id: trigger_id.to_string(),
            trigger,
            code: code.to_string(),
            message: message.to_string(),
        };
        let window_ready = {
            let mut inner = match self.inner.lock() {
                Ok(value) => value,
                Err(_) => return,
            };
            let owns_trigger = inner
                .current_trigger
                .as_ref()
                .map(|value| value.trigger_id == trigger_id)
                .unwrap_or(false)
                || inner
                    .pending_trigger
                    .as_ref()
                    .map(|value| value.trigger_id == trigger_id)
                    .unwrap_or(false)
                || matches!(
                    inner.pending_event.as_ref(),
                    Some(PendingSelectionEvent::Unavailable(value))
                        if value.trigger_id == trigger_id
                );
            if !owns_trigger && inner.window_ready {
                return;
            }
            if !preserve_trigger {
                inner.current_trigger = None;
            }
            inner.pending_event = if inner.window_ready {
                None
            } else {
                Some(PendingSelectionEvent::Unavailable(notice.clone()))
            };
            if inner.window_ready {
                inner.pending_trigger = None;
            }
            inner.window_ready
        };
        diagnostics::warn(format!(
            "selection.capture.failed trigger={trigger:?} code={code}"
        ));
        if window_ready {
            if let Some(app) = self.app_handle() {
                let _ = app.emit(SELECTION_UNAVAILABLE_EVENT, &notice);
            }
            if trigger == SelectionTrigger::Shortcut {
                self.show_window(None);
            }
        } else if code != "selection_window_unavailable" {
            self.ensure_window(trigger_id, trigger);
        }
    }

    #[cfg(windows)]
    fn enqueue_prepared(&self, prepared: PreparedSelection) {
        self.send_worker(WorkerCommand::Prepared(prepared));
    }

    #[cfg(windows)]
    fn current_generation(&self) -> u64 {
        self.inner
            .lock()
            .map(|inner| inner.selection_generation)
            .unwrap_or_default()
    }

    #[cfg(windows)]
    fn owns_generation(&self, generation: u64) -> bool {
        self.current_generation() == generation
    }

    #[cfg(windows)]
    fn publish_prepared_notice(&self, prepared: &PreparedSelection) {
        self.publish_trigger(
            prepared.trigger_id.clone(),
            prepared.trigger,
            prepared.anchor.clone(),
        );
    }

    #[cfg(windows)]
    fn owns_current_trigger(&self, trigger_id: &str) -> bool {
        let inner = match self.inner.lock() {
            Ok(value) => value,
            Err(_) => return false,
        };
        inner
            .current_trigger
            .as_ref()
            .map(|notice| notice.trigger_id == trigger_id)
            .unwrap_or(false)
    }

    #[cfg(windows)]
    fn has_current_automatic_trigger(&self) -> bool {
        self.inner
            .lock()
            .map(|inner| {
                inner
                    .current_trigger
                    .as_ref()
                    .map(|notice| notice.trigger == SelectionTrigger::Automatic)
                    .unwrap_or(false)
            })
            .unwrap_or(false)
    }

    #[cfg(windows)]
    fn current_trigger_anchor(&self, trigger_id: &str) -> Option<SelectionAnchor> {
        self.inner
            .lock()
            .ok()?
            .current_trigger
            .as_ref()
            .and_then(|notice| (notice.trigger_id == trigger_id).then(|| notice.anchor.clone()))?
    }

    fn publish_trigger(
        &self,
        trigger_id: String,
        trigger: SelectionTrigger,
        anchor: Option<SelectionAnchor>,
    ) {
        let notice = SelectionTriggerNotice {
            trigger_id,
            trigger,
            anchor,
        };
        let previous_id = {
            let mut inner = match self.inner.lock() {
                Ok(value) => value,
                Err(_) => return,
            };
            inner.current_trigger = Some(notice.clone());
            let previous_id = inner.latest.take().map(|item| item.payload.request_id);
            inner.last_signature = None;
            inner.pending_event = None;
            if inner.window_ready {
                inner.pending_trigger = None;
            } else {
                inner.pending_trigger = Some(notice.clone());
            }
            (previous_id, inner.window_ready)
        };
        if let Some(request_id) = previous_id.0 {
            if let Ok(mut cancellations) = self.cancellations.lock() {
                if let Some(token) = cancellations.remove(&request_id) {
                    token.cancel();
                }
            }
        }

        diagnostics::info(format!(
            "selection.trigger.ready trigger_id={} trigger={:?} anchor_present={}",
            notice.trigger_id,
            notice.trigger,
            notice.anchor.is_some()
        ));
        if previous_id.1 {
            if let Some(app) = self.app_handle() {
                let _ = app.emit(SELECTION_TRIGGER_EVENT, &notice);
            }
            self.show_for_trigger(&notice);
        } else {
            self.ensure_window(&notice.trigger_id, notice.trigger);
        }
    }

    fn show_for_trigger(&self, notice: &SelectionTriggerNotice) {
        if notice.trigger == SelectionTrigger::Shortcut {
            self.show_window(notice.anchor.as_ref());
        } else {
            self.show_trigger(notice.anchor.as_ref());
        }
    }

    fn publish_candidate(&self, candidate: SelectionCandidate) -> bool {
        let source_text = candidate.source_text.trim().to_string();
        if source_text.is_empty() {
            return false;
        }
        let source_chars = source_text.chars().count();
        let mut inner = match self.inner.lock() {
            Ok(value) => value,
            Err(_) => return false,
        };
        if inner
            .current_trigger
            .as_ref()
            .map(|notice| notice.trigger_id.as_str())
            != Some(candidate.trigger_id.as_str())
        {
            return false;
        }
        let signature = SelectionSignature {
            source_text: source_text.clone(),
            anchor: candidate.anchor.clone(),
        };
        if inner.last_signature.as_ref() == Some(&signature) {
            return false;
        }
        inner.current_trigger = None;
        inner.last_signature = Some(signature);
        let request_id = Uuid::new_v4().to_string();
        let previous_id = inner
            .latest
            .as_ref()
            .map(|item| item.payload.request_id.clone());
        let payload = SelectionRequestPayload {
            request_id: request_id.clone(),
            source_text,
            source_language: inner.source_language.clone(),
            target_language: inner.target_language.clone(),
            trigger: candidate.trigger,
            anchor: candidate.anchor,
        };
        let notice = SelectionNotice {
            request_id: request_id.clone(),
            trigger_id: candidate.trigger_id,
            trigger: payload.trigger,
            anchor: payload.anchor.clone(),
        };
        let window_ready = inner.window_ready;
        inner.latest = Some(StoredSelection {
            payload,
            created_at: Instant::now(),
        });
        inner.pending_event = if window_ready {
            inner.pending_trigger = None;
            None
        } else {
            Some(PendingSelectionEvent::Available(notice.clone()))
        };
        drop(inner);
        self.cancel_request(previous_id);

        diagnostics::info(format!(
            "selection.publish request_id={} trigger={:?} source_chars={} anchor_present={}",
            request_id,
            notice.trigger,
            source_chars,
            notice.anchor.is_some()
        ));

        if window_ready {
            if let Some(app) = self.app_handle() {
                let _ = app.emit(SELECTION_EVENT, &notice);
            }
            self.show_window(notice.anchor.as_ref());
        } else {
            self.ensure_window(&notice.trigger_id, candidate.trigger);
        }
        true
    }

    fn ensure_window(&self, trigger_id: &str, trigger: SelectionTrigger) {
        let Some(app) = self.app_handle() else { return };
        if app.get_webview_window("selection").is_some() {
            return;
        }
        let result =
            WebviewWindowBuilder::new(&app, "selection", WebviewUrl::App("index.html".into()))
                .title("Lilt")
                .inner_size(
                    DEFAULT_SELECTION_WINDOW_WIDTH as f64,
                    DEFAULT_SELECTION_WINDOW_HEIGHT as f64,
                )
                .min_inner_size(
                    MIN_SELECTION_WINDOW_WIDTH as f64,
                    MIN_SELECTION_WINDOW_HEIGHT as f64,
                )
                .decorations(false)
                .transparent(true)
                .shadow(false)
                .resizable(true)
                .always_on_top(true)
                .skip_taskbar(true)
                .focused(false)
                .visible(false)
                .background_color(Color(0, 0, 0, 0))
                .build();
        if let Err(error) = result {
            self.emit_unavailable(
                trigger_id,
                trigger,
                "selection_window_unavailable",
                &format!("无法创建划词浮窗：{error}"),
            );
        }
    }

    fn show_trigger(&self, anchor: Option<&SelectionAnchor>) {
        self.show_at(anchor, 48.0, 48.0, false, true);
    }

    fn show_window(&self, anchor: Option<&SelectionAnchor>) {
        let (width, height) = self.window_size();
        self.show_at(anchor, width as f64, height as f64, true, false);
    }

    fn show_at(
        &self,
        anchor: Option<&SelectionAnchor>,
        desired_width: f64,
        desired_height: f64,
        resizable: bool,
        trigger: bool,
    ) {
        let Some(app) = self.app_handle() else { return };
        let Some(window) = app.get_webview_window("selection") else {
            return;
        };
        let min_size = if resizable {
            LogicalSize::new(
                MIN_SELECTION_WINDOW_WIDTH as f64,
                MIN_SELECTION_WINDOW_HEIGHT as f64,
            )
        } else {
            LogicalSize::new(48.0, 48.0)
        };
        let _ = window.set_min_size(Some(min_size));
        let _ = window.set_resizable(resizable);
        let _ = window.set_size(LogicalSize::new(desired_width, desired_height));
        let size = window.inner_size().unwrap_or_else(|_| {
            tauri::PhysicalSize::new(
                desired_width.max(1.0) as u32,
                desired_height.max(1.0) as u32,
            )
        });
        let monitor_point = anchor.map(|value| {
            if trigger {
                (value.x, value.y)
            } else {
                (
                    value.x,
                    value.y.saturating_add(value.height).saturating_add(8),
                )
            }
        });
        let monitor = monitor_point
            .and_then(|(x, y)| window.monitor_from_point(x as f64, y as f64).ok().flatten())
            .or_else(|| window.current_monitor().ok().flatten())
            .or_else(|| window.primary_monitor().ok().flatten());
        if let Some(monitor) = monitor {
            let work_area = monitor.work_area();
            let work_area = WindowWorkArea {
                x: work_area.position.x,
                y: work_area.position.y,
                width: work_area.size.width.min(i32::MAX as u32) as i32,
                height: work_area.size.height.min(i32::MAX as u32) as i32,
            };
            let (desired_x, desired_y) = selection_window_position(
                anchor,
                (
                    size.width.min(i32::MAX as u32) as i32,
                    size.height.min(i32::MAX as u32) as i32,
                ),
                work_area,
                trigger,
            );
            let position = tauri::PhysicalPosition::new(desired_x, desired_y);
            let _ = window.set_position(position);
        }
        let _ = window.show();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct WindowWorkArea {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
}

fn selection_window_position(
    anchor: Option<&SelectionAnchor>,
    window_size: (i32, i32),
    work_area: WindowWorkArea,
    trigger: bool,
) -> (i32, i32) {
    let window_width = i64::from(window_size.0.max(1));
    let window_height = i64::from(window_size.1.max(1));
    let work_x = i64::from(work_area.x);
    let work_y = i64::from(work_area.y);
    let work_width = i64::from(work_area.width.max(0));
    let work_height = i64::from(work_area.height.max(0));
    let max_x = work_x + (work_width - window_width).max(0);
    let max_y = work_y + (work_height - window_height).max(0);
    let work_right = work_x + work_width;

    let (desired_x, desired_y) = match anchor {
        Some(anchor) if trigger => {
            let anchor_x = i64::from(anchor.x);
            let anchor_y = i64::from(anchor.y);
            let anchor_width = i64::from(anchor.width.max(0));
            let anchor_height = i64::from(anchor.height.max(0));
            let right_x = anchor_x + anchor_width + 8;
            let aligned_y = anchor_y + (anchor_height - window_height).max(0) / 2;
            if right_x + window_width <= work_right {
                (right_x, aligned_y)
            } else {
                (anchor_x, anchor_y + anchor_height + 8)
            }
        }
        Some(anchor) => (
            i64::from(anchor.x),
            i64::from(anchor.y) + i64::from(anchor.height) + 8,
        ),
        None => (
            work_x + (work_width - window_width).max(0) / 2,
            work_y + (work_height - window_height).max(0) / 3,
        ),
    };

    (
        desired_x
            .clamp(work_x, max_x)
            .clamp(i32::MIN as i64, i32::MAX as i64) as i32,
        desired_y
            .clamp(work_y, max_y)
            .clamp(i32::MIN as i64, i32::MAX as i64) as i32,
    )
}

fn recently_resized(last_resized_at: Option<Instant>) -> bool {
    last_resized_at
        .map(|resized_at| resized_at.elapsed() < RESIZE_FOCUS_GRACE)
        .unwrap_or(false)
}

fn normalize_shortcut(shortcut: &str) -> String {
    shortcut
        .split('+')
        .map(|part| match part.trim().to_ascii_lowercase().as_str() {
            "ctrl" | "control" | "mod" => "Control".to_string(),
            "shift" => "Shift".to_string(),
            "alt" | "option" => "Alt".to_string(),
            "super" | "meta" | "cmd" | "command" => "Super".to_string(),
            value => value.to_string(),
        })
        .collect::<Vec<_>>()
        .join("+")
}

fn worker_loop(receiver: mpsc::Receiver<WorkerCommand>, service: SelectionService) {
    #[cfg(windows)]
    {
        windows_worker_loop(receiver, service);
    }
    #[cfg(not(windows))]
    {
        service.set_uia_status(false, Some("划词功能仅支持 Windows".to_string()));
        while let Ok(command) = receiver.recv() {
            match command {
                WorkerCommand::ActivateTrigger { .. } => {}
                WorkerCommand::ReadFocused { .. } => {}
                WorkerCommand::Shutdown => break,
                _ => {}
            }
        }
    }
}

#[cfg(windows)]
fn windows_worker_loop(receiver: mpsc::Receiver<WorkerCommand>, service: SelectionService) {
    use uiautomation::{
        UIAutomation,
        events::{CustomEventHandlerFn, UIEventHandler, UIEventType},
        types::TreeScope,
    };
    use windows::Win32::UI::Input::KeyboardAndMouse::VK_LBUTTON;

    let automation = match UIAutomation::new() {
        Ok(value) => Some(value),
        Err(error) => {
            service.set_uia_status(false, Some(format!("UI Automation 初始化失败：{error}")));
            None
        }
    };
    let root = automation
        .as_ref()
        .and_then(|value| match value.get_root_element() {
            Ok(root) => Some(root),
            Err(error) => {
                service.set_uia_status(false, Some(format!("UI Automation 根元素不可用：{error}")));
                None
            }
        });
    let walker = automation
        .as_ref()
        .and_then(|value| value.create_tree_walker().ok());
    service.set_uia_status(
        root.is_some() && walker.is_some(),
        if root.is_some() && walker.is_some() {
            None
        } else {
            Some("UI Automation 当前不可用，将使用剪贴板兜底".to_string())
        },
    );

    let mut handler: Option<UIEventHandler> = None;
    let mut automatic = false;
    let mut prepared: Option<PreparedSelection> = None;
    let mut pending_prepared: Option<(PreparedSelection, Instant)> = None;
    let mut pending_mouse_release: Option<MouseSelectionRelease> = None;
    let mut mouse_operation: Option<MouseSelectionOperation> = None;
    let mut left_button_down = false;
    let mut next_focus_poll = Instant::now();

    loop {
        let now = Instant::now();
        if let Some((value, deadline)) = pending_prepared.take() {
            if deadline <= now && service.owns_generation(value.generation) {
                service.publish_prepared_notice(&value);
                prepared = Some(value);
            } else if deadline > now && service.owns_generation(value.generation) {
                pending_prepared = Some((value, deadline));
            }
        }

        if automatic {
            let button_down = key_is_down(VK_LBUTTON.0 as i32);
            if button_down && !left_button_down {
                left_button_down = true;
                pending_mouse_release = None;
                mouse_operation = cursor_position().and_then(|(x, y)| {
                    let process_id = process_id_at_point(x, y);
                    if process_id == Some(std::process::id()) {
                        return None;
                    }
                    service.invalidate_for_new_selection();
                    prepared = None;
                    pending_prepared = None;
                    Some(MouseSelectionOperation { process_id })
                });
            } else if !button_down && left_button_down {
                left_button_down = false;
                if let Some(operation) = mouse_operation.take() {
                    if operation.process_id != Some(std::process::id()) {
                        pending_mouse_release = Some(MouseSelectionRelease {
                            process_id: operation.process_id,
                            anchor: cursor_position().map(point_selection_anchor),
                            generation: service.current_generation(),
                            deadline: now + MOUSE_RELEASE_DELAY,
                        });
                    }
                }
            }

            if now >= next_focus_poll {
                next_focus_poll = now + FOCUS_POLL_INTERVAL;
                if let (Some(automation), Some(walker)) = (automation.as_ref(), walker.as_ref()) {
                    if let Ok(element) = automation.get_focused_element() {
                        if let Some(value) = prepare_element(
                            &service,
                            &element,
                            SelectionTrigger::Automatic,
                            walker,
                            None,
                            cursor_position().map(point_selection_anchor),
                        ) {
                            accept_automatic_prepared(
                                &service,
                                value,
                                &mut pending_prepared,
                                &mut prepared,
                            );
                            pending_mouse_release = None;
                        }
                    }
                }
            }
        } else {
            left_button_down = false;
            mouse_operation = None;
            pending_mouse_release = None;
        }

        if let Some(release) = pending_mouse_release.take() {
            if release.deadline > now {
                pending_mouse_release = Some(release);
            } else if automatic
                && release.process_id != Some(std::process::id())
                && service.owns_generation(release.generation)
            {
                let has_prepared = pending_prepared
                    .as_ref()
                    .map(|(value, _)| value.generation == release.generation)
                    .unwrap_or(false)
                    || prepared
                        .as_ref()
                        .map(|value| value.generation == release.generation)
                        .unwrap_or(false);
                if !has_prepared {
                    let focus_prepared = automation.as_ref().zip(walker.as_ref()).and_then(
                        |(automation, walker)| {
                            automation.get_focused_element().ok().and_then(|element| {
                                prepare_element(
                                    &service,
                                    &element,
                                    SelectionTrigger::Automatic,
                                    walker,
                                    None,
                                    release.anchor.clone(),
                                )
                            })
                        },
                    );
                    if let Some(value) = focus_prepared {
                        accept_automatic_prepared(
                            &service,
                            value,
                            &mut pending_prepared,
                            &mut prepared,
                        );
                    } else {
                        match capture_clipboard_selection() {
                            Ok(source_text) if service.owns_generation(release.generation) => {
                                let value = PreparedSelection {
                                    trigger_id: Uuid::new_v4().to_string(),
                                    generation: release.generation,
                                    trigger: SelectionTrigger::Automatic,
                                    anchor: release.anchor.clone(),
                                    source: PreparedSelectionSource::Clipboard(source_text),
                                };
                                accept_automatic_prepared(
                                    &service,
                                    value,
                                    &mut pending_prepared,
                                    &mut prepared,
                                );
                            }
                            Ok(_) => {}
                            Err(error) => diagnostics::warn(format!(
                                "selection.clipboard.failed trigger=automatic code={}",
                                error.code
                            )),
                        }
                    }
                }
            }
        }

        match receiver.recv_timeout(Duration::from_millis(40)) {
            Ok(WorkerCommand::SetAutomatic(enabled)) => {
                if enabled == automatic {
                    continue;
                }
                service.invalidate_for_new_selection();
                prepared = None;
                pending_prepared = None;
                pending_mouse_release = None;
                mouse_operation = None;
                left_button_down = false;
                if enabled {
                    if let (Some(automation), Some(root), Some(walker)) =
                        (automation.as_ref(), root.as_ref(), walker.as_ref())
                    {
                        let service_for_event = service.clone();
                        let walker_for_event = walker.clone();
                        let callback: Box<CustomEventHandlerFn> =
                            Box::new(move |sender, _event| {
                                if sender.get_process_id().ok() == Some(std::process::id()) {
                                    return Ok(());
                                }
                                if let Some(value) = prepare_element(
                                    &service_for_event,
                                    sender,
                                    SelectionTrigger::Automatic,
                                    &walker_for_event,
                                    None,
                                    cursor_position().map(point_selection_anchor),
                                ) {
                                    service_for_event.enqueue_prepared(value);
                                }
                                Ok(())
                            });
                        let event_handler = UIEventHandler::from(callback);
                        match automation.add_automation_event_handler(
                            UIEventType::Text_TextSelectionChanged,
                            root,
                            TreeScope::Subtree,
                            None,
                            &event_handler,
                        ) {
                            Ok(()) => {
                                handler = Some(event_handler);
                                service.set_uia_status(true, None);
                            }
                            Err(error) => service
                                .set_uia_status(false, Some(format!("自动选区监听失败：{error}"))),
                        }
                    }
                    automatic = true;
                } else {
                    if let Some(event_handler) = handler.take() {
                        if let (Some(automation), Some(root)) = (automation.as_ref(), root.as_ref())
                        {
                            let _ = automation.remove_automation_event_handler(
                                UIEventType::Text_TextSelectionChanged,
                                root,
                                &event_handler,
                            );
                        }
                    }
                    automatic = false;
                    service.set_uia_status(root.is_some() && walker.is_some(), None);
                }
            }
            Ok(WorkerCommand::ReadFocused { trigger_id }) if !automatic => {
                if !service.owns_current_trigger(&trigger_id) {
                    continue;
                }
                let generation = service.current_generation();
                let uia_candidate =
                    automation
                        .as_ref()
                        .zip(walker.as_ref())
                        .and_then(|(automation, walker)| {
                            automation.get_focused_element().ok().and_then(|element| {
                                prepare_element(
                                    &service,
                                    &element,
                                    SelectionTrigger::Shortcut,
                                    walker,
                                    Some(trigger_id.clone()),
                                    cursor_position().map(point_selection_anchor),
                                )
                                .and_then(|prepared| read_prepared_selection(&prepared))
                            })
                        });
                if let Some(candidate) = uia_candidate {
                    if service.owns_generation(generation)
                        && service.owns_current_trigger(&trigger_id)
                    {
                        let _ = service.publish_candidate(candidate);
                        continue;
                    }
                }

                if let Err(error) = wait_for_shortcut_release() {
                    service.emit_unavailable(
                        &trigger_id,
                        SelectionTrigger::Shortcut,
                        error.code,
                        &error.message,
                    );
                    continue;
                }
                if !service.owns_generation(generation)
                    || !service.owns_current_trigger(&trigger_id)
                {
                    continue;
                }
                let anchor = cursor_position().map(|(x, y)| SelectionAnchor {
                    x,
                    y,
                    width: 0,
                    height: 0,
                });
                match capture_clipboard_selection() {
                    Ok(source_text) => {
                        let candidate = SelectionCandidate {
                            trigger_id,
                            source_text,
                            anchor,
                            trigger: SelectionTrigger::Shortcut,
                        };
                        if service.owns_current_trigger(&candidate.trigger_id) {
                            let _ = service.publish_candidate(candidate);
                        }
                    }
                    Err(error) => service.emit_unavailable(
                        &trigger_id,
                        SelectionTrigger::Shortcut,
                        error.code,
                        &error.message,
                    ),
                }
            }
            Ok(WorkerCommand::Prepared(value))
                if value.trigger == SelectionTrigger::Automatic && automatic =>
            {
                accept_automatic_prepared(&service, value, &mut pending_prepared, &mut prepared);
            }
            Ok(WorkerCommand::ActivateTrigger { trigger_id }) => {
                if !service.owns_current_trigger(&trigger_id) {
                    diagnostics::info(format!(
                        "selection.trigger.read.discarded trigger_id={trigger_id}"
                    ));
                    continue;
                }

                let anchor = service.current_trigger_anchor(&trigger_id);
                let mut candidate = prepared
                    .as_ref()
                    .filter(|value| value.trigger_id == trigger_id)
                    .and_then(read_prepared_selection);
                let mut failure = candidate.is_none().then(|| {
                    SelectionCaptureFailure::new("selection_read_failed", "点击后无法读取当前选区")
                });

                if candidate.is_none() && service.owns_current_trigger(&trigger_id) {
                    match capture_clipboard_selection() {
                        Ok(source_text) => {
                            candidate = Some(SelectionCandidate {
                                trigger_id: trigger_id.clone(),
                                source_text,
                                anchor,
                                trigger: SelectionTrigger::Automatic,
                            });
                            failure = None;
                        }
                        Err(error) => failure = Some(error),
                    }
                }

                if let Some(candidate) = candidate {
                    if service.owns_current_trigger(&trigger_id)
                        && service.publish_candidate(candidate)
                    {
                        prepared = None;
                        diagnostics::info(format!(
                            "selection.trigger.read.completed trigger_id={trigger_id}"
                        ));
                    } else if service.owns_current_trigger(&trigger_id) {
                        service.publish_activation_unavailable(
                            &trigger_id,
                            SelectionTrigger::Automatic,
                            "selection_read_failed",
                            "点击后无法归属当前选区读取结果",
                        );
                    } else {
                        diagnostics::info(format!(
                            "selection.trigger.read.discarded trigger_id={trigger_id}"
                        ));
                    }
                } else if service.owns_current_trigger(&trigger_id) {
                    let error = failure.unwrap_or_else(|| {
                        SelectionCaptureFailure::new(
                            "selection_read_failed",
                            "点击后无法读取当前选区",
                        )
                    });
                    service.publish_activation_unavailable(
                        &trigger_id,
                        SelectionTrigger::Automatic,
                        error.code,
                        &error.message,
                    );
                }
            }
            Ok(WorkerCommand::ClearPending) => {
                prepared = None;
                pending_prepared = None;
                pending_mouse_release = None;
            }
            Ok(WorkerCommand::Shutdown) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Ok(WorkerCommand::ReadFocused { .. }) => {}
            Ok(WorkerCommand::Prepared(_)) => {}
        }
    }
    if let Some(event_handler) = handler {
        if let (Some(automation), Some(root)) = (automation.as_ref(), root.as_ref()) {
            let _ = automation.remove_automation_event_handler(
                UIEventType::Text_TextSelectionChanged,
                root,
                &event_handler,
            );
        }
    }
}

#[cfg(windows)]
fn accept_automatic_prepared(
    service: &SelectionService,
    mut value: PreparedSelection,
    pending_prepared: &mut Option<(PreparedSelection, Instant)>,
    prepared: &mut Option<PreparedSelection>,
) {
    if !service.owns_generation(value.generation) {
        return;
    }
    if service.has_current_automatic_trigger() {
        return;
    }
    let duplicate = pending_prepared
        .as_ref()
        .map(|(current, _)| same_prepared_selection(current, &value))
        .unwrap_or(false)
        || prepared
            .as_ref()
            .map(|current| same_prepared_selection(current, &value))
            .unwrap_or(false);
    if duplicate {
        return;
    }
    service.invalidate_for_new_selection();
    value.generation = service.current_generation();
    *prepared = None;
    *pending_prepared = Some((value, Instant::now() + AUTOMATIC_DEBOUNCE));
}

#[cfg(windows)]
fn prepare_element(
    service: &SelectionService,
    element: &uiautomation::UIElement,
    trigger: SelectionTrigger,
    walker: &uiautomation::UITreeWalker,
    trigger_id: Option<String>,
    fallback_anchor: Option<SelectionAnchor>,
) -> Option<PreparedSelection> {
    use uiautomation::patterns::UITextPattern;
    use uiautomation::types::TextPatternRangeEndpoint;

    let trigger_id = trigger_id.unwrap_or_else(|| Uuid::new_v4().to_string());
    let mut current = element.clone();
    for _ in 0..=MAX_SELECTION_ANCESTORS {
        if current.get_process_id().ok() == Some(std::process::id()) {
            return None;
        }
        if let Ok(pattern) = current.get_pattern::<UITextPattern>() {
            if let Ok(ranges) = pattern.get_selection() {
                let ranges = ranges
                    .into_iter()
                    .filter(|range| {
                        range
                            .compare_endpoints(
                                TextPatternRangeEndpoint::Start,
                                range,
                                TextPatternRangeEndpoint::End,
                            )
                            .map(|distance| distance < 0)
                            .unwrap_or(false)
                    })
                    .collect::<Vec<_>>();
                if !ranges.is_empty() {
                    let anchor = selection_anchor_from_text_ranges(&ranges)
                        .or(fallback_anchor)
                        .or_else(|| {
                            ranges.first().and_then(|range| {
                                range
                                    .get_enclosing_element()
                                    .ok()
                                    .and_then(|value| value.get_bounding_rectangle().ok())
                                    .and_then(|rect| {
                                        selection_anchor_from_bounding_rectangles(&[
                                            rect.get_left() as f64,
                                            rect.get_top() as f64,
                                            rect.get_width() as f64,
                                            rect.get_height() as f64,
                                        ])
                                    })
                            })
                        });
                    let mut agile_ranges = Vec::with_capacity(ranges.len());
                    for range in ranges {
                        let value = windows::core::AgileReference::new(range.as_ref()).ok()?;
                        agile_ranges.push(value);
                    }
                    return Some(PreparedSelection {
                        trigger_id,
                        generation: service.current_generation(),
                        trigger,
                        anchor,
                        source: PreparedSelectionSource::UiAutomation(agile_ranges),
                    });
                }
            }
        }
        current = walker.get_parent(&current).ok()?;
    }
    None
}

#[cfg(windows)]
fn point_selection_anchor((x, y): (i32, i32)) -> SelectionAnchor {
    SelectionAnchor {
        x,
        y,
        width: 0,
        height: 0,
    }
}

#[cfg(windows)]
fn selection_anchor_from_text_ranges(
    ranges: &[uiautomation::patterns::UITextRange],
) -> Option<SelectionAnchor> {
    use windows::Win32::System::Ole::SafeArrayDestroy;

    let mut rectangles = Vec::new();
    for range in ranges {
        let Ok(array) = (unsafe { range.as_ref().GetBoundingRectangles() }) else {
            continue;
        };
        if array.is_null() {
            continue;
        }
        if let Some(values) = safe_array_f64_values(array) {
            rectangles.extend(values);
        }
        unsafe {
            let _ = SafeArrayDestroy(array);
        }
    }
    selection_anchor_from_bounding_rectangles(&rectangles)
}

#[cfg(windows)]
fn safe_array_f64_values(array: *const windows::Win32::System::Com::SAFEARRAY) -> Option<Vec<f64>> {
    use std::{ffi::c_void, ptr::null_mut, slice};
    use windows::Win32::System::Ole::{
        SafeArrayAccessData, SafeArrayGetDim, SafeArrayGetLBound, SafeArrayGetUBound,
        SafeArrayUnaccessData,
    };

    unsafe {
        if array.is_null() || SafeArrayGetDim(array) != 1 {
            return None;
        }
        let lower = SafeArrayGetLBound(array, 1).ok()?;
        let upper = SafeArrayGetUBound(array, 1).ok()?;
        let count = i64::from(upper) - i64::from(lower) + 1;
        if count <= 0 || count > 4096 || count % 4 != 0 {
            return None;
        }

        let mut data: *mut c_void = null_mut();
        SafeArrayAccessData(array, &mut data).ok()?;
        let values = slice::from_raw_parts(data.cast::<f64>(), count as usize).to_vec();
        let _ = SafeArrayUnaccessData(array);
        Some(values)
    }
}

#[cfg(windows)]
fn selection_anchor_from_bounding_rectangles(values: &[f64]) -> Option<SelectionAnchor> {
    let mut bounds: Option<(f64, f64, f64, f64)> = None;
    for rectangle in values.chunks_exact(4) {
        let [left, top, width, height] = rectangle else {
            continue;
        };
        if !left.is_finite()
            || !top.is_finite()
            || !width.is_finite()
            || !height.is_finite()
            || *width <= 0.0
            || *height <= 0.0
        {
            continue;
        }
        let right = *left + *width;
        let bottom = *top + *height;
        if !right.is_finite() || !bottom.is_finite() {
            continue;
        }
        bounds = Some(match bounds {
            Some((min_left, min_top, max_right, max_bottom)) => (
                min_left.min(*left),
                min_top.min(*top),
                max_right.max(right),
                max_bottom.max(bottom),
            ),
            None => (*left, *top, right, bottom),
        });
    }

    let (left, top, right, bottom) = bounds?;
    let x = finite_i32(left)?;
    let y = finite_i32(top)?;
    let right = finite_i32(right)?;
    let bottom = finite_i32(bottom)?;
    let width = right.checked_sub(x)?;
    let height = bottom.checked_sub(y)?;
    (width > 0 && height > 0).then_some(SelectionAnchor {
        x,
        y,
        width,
        height,
    })
}

#[cfg(windows)]
fn finite_i32(value: f64) -> Option<i32> {
    (value.is_finite() && value >= f64::from(i32::MIN) && value <= f64::from(i32::MAX))
        .then_some(value.round() as i32)
}

#[cfg(windows)]
fn same_prepared_selection(left: &PreparedSelection, right: &PreparedSelection) -> bool {
    use uiautomation::patterns::UITextRange;

    match (&left.source, &right.source) {
        (
            PreparedSelectionSource::Clipboard(left_text),
            PreparedSelectionSource::Clipboard(right_text),
        ) => left_text == right_text && left.anchor == right.anchor,
        (
            PreparedSelectionSource::UiAutomation(left_ranges),
            PreparedSelectionSource::UiAutomation(right_ranges),
        ) => {
            if left_ranges.len() != right_ranges.len() {
                return false;
            }
            left_ranges
                .iter()
                .zip(right_ranges.iter())
                .all(|(left_range, right_range)| {
                    let Ok(left_range) = left_range.resolve() else {
                        return false;
                    };
                    let Ok(right_range) = right_range.resolve() else {
                        return false;
                    };
                    UITextRange::from(left_range)
                        .compare(&UITextRange::from(right_range))
                        .unwrap_or(false)
                })
        }
        _ => false,
    }
}

#[cfg(windows)]
fn read_prepared_selection(prepared: &PreparedSelection) -> Option<SelectionCandidate> {
    let source_text = match &prepared.source {
        PreparedSelectionSource::Clipboard(text) => text.clone(),
        PreparedSelectionSource::UiAutomation(ranges) => {
            use uiautomation::patterns::UITextRange;
            let mut text = String::new();
            for agile_range in ranges {
                let range = UITextRange::from(agile_range.resolve().ok()?);
                text.push_str(&range.get_text(-1).ok()?);
            }
            text
        }
    };
    if source_text.trim().is_empty() {
        return None;
    }
    Some(SelectionCandidate {
        trigger_id: prepared.trigger_id.clone(),
        source_text,
        anchor: prepared.anchor.clone(),
        trigger: prepared.trigger,
    })
}

#[cfg(windows)]
fn key_is_down(key: i32) -> bool {
    use windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;

    unsafe { GetAsyncKeyState(key) < 0 }
}

#[cfg(windows)]
fn cursor_position() -> Option<(i32, i32)> {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;

    let mut point = POINT { x: 0, y: 0 };
    unsafe { GetCursorPos(&mut point).ok()? };
    Some((point.x, point.y))
}

#[cfg(windows)]
fn process_id_at_point(x: i32, y: i32) -> Option<u32> {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::UI::WindowsAndMessaging::{GetWindowThreadProcessId, WindowFromPoint};

    let window = unsafe { WindowFromPoint(POINT { x, y }) };
    if window.0.is_null() {
        return None;
    }
    let mut process_id = 0;
    let thread_id = unsafe { GetWindowThreadProcessId(window, Some(&mut process_id)) };
    (thread_id != 0 && process_id != 0).then_some(process_id)
}

#[cfg(windows)]
fn wait_for_shortcut_release() -> Result<(), SelectionCaptureFailure> {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        VK_CONTROL, VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT,
    };

    let modifiers = [
        VK_CONTROL.0 as i32,
        VK_SHIFT.0 as i32,
        VK_MENU.0 as i32,
        VK_LWIN.0 as i32,
        VK_RWIN.0 as i32,
    ];
    let deadline = Instant::now() + SHORTCUT_RELEASE_TIMEOUT;
    while modifiers.iter().any(|key| key_is_down(*key)) {
        if Instant::now() >= deadline {
            return Err(SelectionCaptureFailure::new(
                "shortcut_release_timeout",
                "等待快捷键释放超时，未读取剪贴板选区",
            ));
        }
        thread::sleep(Duration::from_millis(10));
    }
    Ok(())
}

#[cfg(windows)]
fn clipboard_has_unsafe_formats(
    clipboard: &uiautomation::clipboards::Clipboard,
) -> Result<(), SelectionCaptureFailure> {
    use windows::Win32::System::DataExchange::EnumClipboardFormats;

    let mut format = 0;
    loop {
        let next = unsafe { EnumClipboardFormats(format) };
        if next == 0 {
            break;
        }
        if matches!(
            next,
            2 | 3 | 9 | 10 | 14 | 15 | 128 | 129 | 130 | 131 | 142
                | 512..=767
                | 768..=1023
        ) {
            return Err(SelectionCaptureFailure::new(
                "clipboard_format_unsafe",
                "当前剪贴板包含无法安全恢复的格式",
            ));
        }
        format = next;
    }
    let _ = clipboard;
    Ok(())
}

#[cfg(windows)]
fn capture_clipboard_selection() -> Result<String, SelectionCaptureFailure> {
    use uiautomation::clipboards::Clipboard;
    use uiautomation::inputs::Keyboard;

    let snapshot = {
        let clipboard = Clipboard::open().map_err(|error| {
            SelectionCaptureFailure::new(
                "clipboard_open_failed",
                format!("无法打开系统剪贴板：{error}"),
            )
        })?;
        clipboard_has_unsafe_formats(&clipboard)?;
        clipboard.snapshot().map_err(|error| {
            SelectionCaptureFailure::new(
                "clipboard_snapshot_failed",
                format!("无法安全保存系统剪贴板：{error}"),
            )
        })?
    };
    let sentinel = format!("__lilt_selection_sentinel_{}__", Uuid::new_v4());
    let capture_result = (|| {
        {
            let clipboard = Clipboard::open().map_err(|error| {
                SelectionCaptureFailure::new(
                    "clipboard_open_failed",
                    format!("无法准备系统剪贴板：{error}"),
                )
            })?;
            clipboard.set_text(&sentinel).map_err(|error| {
                SelectionCaptureFailure::new(
                    "clipboard_write_failed",
                    format!("无法准备剪贴板选区读取：{error}"),
                )
            })?;
        }
        Keyboard::new()
            .interval(0)
            .send_keys("{ctrl}(c)")
            .map_err(|error| {
                SelectionCaptureFailure::new(
                    "clipboard_copy_failed",
                    format!("无法请求目标应用复制选区：{error}"),
                )
            })?;
        let deadline = Instant::now() + CLIPBOARD_CAPTURE_TIMEOUT;
        loop {
            if let Ok(clipboard) = Clipboard::open() {
                if let Ok(text) = clipboard.get_text() {
                    if text != sentinel && !text.trim().is_empty() {
                        return Ok(text);
                    }
                }
            }
            if Instant::now() >= deadline {
                break;
            }
            thread::sleep(CLIPBOARD_POLL_INTERVAL);
        }
        Err(SelectionCaptureFailure::new(
            "no_selection",
            "当前没有可读取的文本选区",
        ))
    })();
    let restore_result = Clipboard::open()
        .map_err(|error| format!("无法重新打开系统剪贴板：{error}"))
        .and_then(|clipboard| {
            clipboard
                .restore(snapshot)
                .map_err(|error| format!("无法恢复系统剪贴板：{error}"))
        });
    match restore_result {
        Ok(()) => capture_result,
        Err(message) => Err(SelectionCaptureFailure::new(
            "clipboard_restore_failed",
            message,
        )),
    }
}

#[cfg(test)]
mod tests {
    #[cfg(windows)]
    use super::selection_anchor_from_bounding_rectangles;
    use super::{
        SelectionAnchor, SelectionService, WindowWorkArea, normalize_shortcut, recently_resized,
        selection_window_position,
    };
    use std::{
        collections::HashMap,
        sync::{Arc, Mutex},
        time::Instant,
    };

    fn test_service() -> SelectionService {
        SelectionService::new(Arc::new(Mutex::new(HashMap::new())))
    }

    #[test]
    fn resize_grace_only_applies_after_a_resize_event() {
        assert!(!recently_resized(None));
        assert!(recently_resized(Some(Instant::now())));
    }

    #[test]
    fn resize_and_focus_gain_cancel_pending_focus_hide_checks() {
        let service = test_service();
        service.handle_focus_lost();
        let after_focus_lost = service.inner.lock().unwrap().focus_lost_generation;
        service.handle_window_resized();
        let after_resize = service.inner.lock().unwrap().focus_lost_generation;
        assert_ne!(after_resize, after_focus_lost);
        service.handle_focus_gained();
        let after_focus_gained = service.inner.lock().unwrap().focus_lost_generation;
        assert_ne!(after_focus_gained, after_resize);
    }

    #[test]
    fn normalize_shortcut_accepts_common_modifier_aliases() {
        assert_eq!(normalize_shortcut(" ctrl + shift + l "), "Control+Shift+l");
        assert_eq!(normalize_shortcut("cmd+alt+k"), "Super+Alt+k");
    }

    #[test]
    fn trigger_window_prefers_the_right_edge_of_the_selection() {
        let anchor = SelectionAnchor {
            x: 100,
            y: 200,
            width: 80,
            height: 24,
        };
        let position = selection_window_position(
            Some(&anchor),
            (48, 48),
            WindowWorkArea {
                x: 0,
                y: 0,
                width: 1200,
                height: 800,
            },
            true,
        );
        assert_eq!(position, (188, 200));
    }

    #[test]
    fn trigger_window_falls_below_and_stays_inside_work_area() {
        let anchor = SelectionAnchor {
            x: 1160,
            y: 740,
            width: 30,
            height: 20,
        };
        let position = selection_window_position(
            Some(&anchor),
            (48, 48),
            WindowWorkArea {
                x: 0,
                y: 0,
                width: 1200,
                height: 800,
            },
            true,
        );
        assert_eq!(position, (1152, 752));
    }

    #[cfg(windows)]
    #[test]
    fn selection_anchor_covers_all_visible_text_rectangles() {
        let anchor = selection_anchor_from_bounding_rectangles(&[
            10.0,
            20.0,
            40.0,
            12.0,
            10.0,
            40.0,
            30.0,
            12.0,
            f64::NAN,
            0.0,
            20.0,
            20.0,
        ]);

        assert_eq!(
            anchor,
            Some(SelectionAnchor {
                x: 10,
                y: 20,
                width: 40,
                height: 32,
            })
        );
    }

    #[cfg(windows)]
    #[test]
    fn selection_anchor_rejects_invalid_rectangles() {
        assert_eq!(
            selection_anchor_from_bounding_rectangles(&[0.0, 0.0, 0.0, 10.0]),
            None
        );
        assert_eq!(
            selection_anchor_from_bounding_rectangles(&[0.0, 0.0, f64::INFINITY, 10.0]),
            None
        );
    }
}
