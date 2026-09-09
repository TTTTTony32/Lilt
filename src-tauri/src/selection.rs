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

#[cfg(windows)]
mod edit;
#[cfg(windows)]
mod provider;

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
const MIN_AUTOMATIC_DRAG_DISTANCE: i32 = 4;
const AUTOMATIC_TRIGGER_READING_WIDTH: f64 = 280.0;
const AUTOMATIC_TRIGGER_READING_HEIGHT: f64 = 72.0;
const AUTOMATIC_TRIGGER_FAILED_WIDTH: f64 = 320.0;
const AUTOMATIC_TRIGGER_FAILED_HEIGHT: f64 = 80.0;
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
    reading_trigger: Option<String>,
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

// Experimental UIA-only mode. Keep the whitelist and protected-copy path in
// place so the experiment can be reverted without changing the routing again.
const CLIPBOARD_SELECTION_ENABLED: bool = false;
const CLIPBOARD_SOURCE_PROCESS_WHITELIST: &[&str] = &["zotero.exe"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SourceComparison {
    Exact,
    PartialOverlap,
    NoOverlap,
    ClipboardUnavailable,
    UiAutomationUnavailable,
    BothUnavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SourceChoice {
    UiAutomation,
    Clipboard,
    Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SourceReconciliation {
    comparison: SourceComparison,
    choice: SourceChoice,
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
    source_context: Option<SelectionSourceContext>,
    standard_edit_window: Option<isize>,
    source: PreparedSelectionSource,
}

#[cfg(windows)]
#[derive(Clone)]
enum PreparedSelectionSource {
    UiAutomation(Vec<AgileTextRange>),
    StandardEdit,
    ClipboardOnActivate,
}

#[cfg(windows)]
#[derive(Clone, Copy)]
struct SelectionSourceContext {
    generation: u64,
    process_id: u32,
    root_window: isize,
    press_point: Option<(i32, i32)>,
    release_point: Option<(i32, i32)>,
    started_at: Instant,
}

#[cfg(windows)]
struct PrepareElementRequest {
    trigger: SelectionTrigger,
    trigger_id: Option<String>,
    fallback_anchor: Option<SelectionAnchor>,
    source_context: Option<SelectionSourceContext>,
    allow_clipboard_fallback: bool,
}

#[cfg(windows)]
struct MouseSelectionOperation {
    context: SelectionSourceContext,
}

#[cfg(windows)]
struct MouseSelectionRelease {
    context: SelectionSourceContext,
    anchor: SelectionAnchor,
    ready_at: Instant,
    probe_until: Instant,
    release_probe_done: bool,
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
                reading_trigger: None,
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
                    self.show_window(notice.anchor.as_ref(), true);
                }
                PendingSelectionEvent::Unavailable(notice) => {
                    if let Some(app) = self.app_handle() {
                        let _ = app.emit(SELECTION_UNAVAILABLE_EVENT, &notice);
                    }
                    if notice.trigger == SelectionTrigger::Shortcut {
                        self.show_window(None, true);
                    } else {
                        self.show_trigger_feedback(
                            pending.as_ref().and_then(|value| value.anchor.as_ref()),
                            AUTOMATIC_TRIGGER_FAILED_WIDTH,
                            AUTOMATIC_TRIGGER_FAILED_HEIGHT,
                        );
                    }
                }
            }
        }
        pending
    }

    pub fn activate_trigger(&self, trigger_id: &str) -> Result<(), String> {
        let (anchor, already_reading) = {
            let mut inner = self
                .inner
                .lock()
                .map_err(|_| "选区状态锁已损坏".to_string())?;
            let Some(notice) = inner.current_trigger.as_ref().filter(|notice| {
                notice.trigger_id == trigger_id && notice.trigger == SelectionTrigger::Automatic
            }) else {
                return Err("划词触发已过期".to_string());
            };
            let anchor = notice.anchor.clone();
            if inner.reading_trigger.as_deref() == Some(trigger_id) {
                (anchor, true)
            } else if inner.reading_trigger.is_some() {
                return Err("另一个划词读取正在进行".to_string());
            } else {
                inner.reading_trigger = Some(trigger_id.to_string());
                (anchor, false)
            }
        };
        if already_reading {
            return Ok(());
        }
        self.show_trigger_feedback(
            anchor.as_ref(),
            AUTOMATIC_TRIGGER_READING_WIDTH,
            AUTOMATIC_TRIGGER_READING_HEIGHT,
        );
        if !self.send_worker(WorkerCommand::ActivateTrigger {
            trigger_id: trigger_id.to_string(),
        }) {
            self.finish_trigger_read(trigger_id);
            return Err("划词工作线程不可用".to_string());
        }
        diagnostics::info(format!(
            "selection.trigger.activate trigger_id={trigger_id}"
        ));
        Ok(())
    }

    fn finish_trigger_read(&self, trigger_id: &str) {
        if let Ok(mut inner) = self.inner.lock()
            && inner.reading_trigger.as_deref() == Some(trigger_id)
        {
            inner.reading_trigger = None;
        }
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
            inner.reading_trigger = None;
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
                    inner.reading_trigger = None;
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
            if !owns_trigger {
                return;
            }
            if inner.reading_trigger.as_deref() == Some(trigger_id) {
                inner.reading_trigger = None;
            }
            if trigger == SelectionTrigger::Shortcut {
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
            (
                inner.window_ready,
                inner
                    .current_trigger
                    .as_ref()
                    .filter(|value| value.trigger_id == trigger_id)
                    .and_then(|value| value.anchor.clone())
                    .or_else(|| {
                        inner
                            .pending_trigger
                            .as_ref()
                            .filter(|value| value.trigger_id == trigger_id)
                            .and_then(|value| value.anchor.clone())
                    }),
            )
        };
        diagnostics::warn(format!(
            "selection.capture.failed trigger={trigger:?} code={code}"
        ));
        if window_ready.0 {
            if let Some(app) = self.app_handle() {
                let _ = app.emit(SELECTION_UNAVAILABLE_EVENT, &notice);
            }
            if trigger == SelectionTrigger::Shortcut {
                self.show_window(None, true);
            } else {
                self.show_trigger_feedback(
                    window_ready.1.as_ref(),
                    AUTOMATIC_TRIGGER_FAILED_WIDTH,
                    AUTOMATIC_TRIGGER_FAILED_HEIGHT,
                );
            }
        } else if code != "selection_window_unavailable" {
            self.ensure_window(trigger_id, trigger);
        }
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
    fn enqueue_prepared(&self, prepared: PreparedSelection) {
        self.send_worker(WorkerCommand::Prepared(prepared));
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
    fn has_automatic_trigger(&self) -> bool {
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
            inner.reading_trigger = None;
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
            self.show_window(notice.anchor.as_ref(), false);
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
        inner.reading_trigger = None;
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
            self.show_window(notice.anchor.as_ref(), true);
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
                .focusable(false)
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
        self.show_at(anchor, 48.0, 48.0, false, false);
    }

    fn show_trigger_feedback(&self, anchor: Option<&SelectionAnchor>, width: f64, height: f64) {
        self.show_at(anchor, width, height, false, false);
    }

    fn show_window(&self, anchor: Option<&SelectionAnchor>, focusable: bool) {
        let (width, height) = self.window_size();
        self.show_at(anchor, width as f64, height as f64, true, focusable);
    }

    fn show_at(
        &self,
        anchor: Option<&SelectionAnchor>,
        desired_width: f64,
        desired_height: f64,
        resizable: bool,
        focusable: bool,
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
        let _ = window.set_focusable(focusable);
        let _ = window.set_size(LogicalSize::new(desired_width, desired_height));
        let size = window.inner_size().unwrap_or_else(|_| {
            tauri::PhysicalSize::new(
                desired_width.max(1.0) as u32,
                desired_height.max(1.0) as u32,
            )
        });
        let desired = if resizable {
            anchor.map(|value| (value.x, value.y + value.height + 8))
        } else {
            anchor.and_then(|value| {
                let monitor = window
                    .monitor_from_point(value.x as f64, value.y as f64)
                    .ok()
                    .flatten()
                    .or_else(|| window.current_monitor().ok().flatten())
                    .or_else(|| window.primary_monitor().ok().flatten());
                monitor.map(|monitor| {
                    let work_area = monitor.work_area();
                    trigger_position(
                        value,
                        size.width as i32,
                        size.height as i32,
                        work_area.position.x,
                        work_area.position.y,
                        work_area.size.width as i32,
                        work_area.size.height as i32,
                    )
                })
            })
        };
        let monitor = desired
            .and_then(|(x, y)| window.monitor_from_point(x as f64, y as f64).ok().flatten())
            .or_else(|| window.current_monitor().ok().flatten())
            .or_else(|| window.primary_monitor().ok().flatten());
        if let Some(monitor) = monitor {
            let work_area = monitor.work_area();
            let min_x = work_area.position.x;
            let min_y = work_area.position.y;
            let max_x = min_x + work_area.size.width as i32 - size.width as i32;
            let max_y = min_y + work_area.size.height as i32 - size.height as i32;
            let (desired_x, desired_y) = desired.unwrap_or((
                min_x + (work_area.size.width as i32 - size.width as i32).max(0) / 2,
                min_y + (work_area.size.height as i32 - size.height as i32).max(0) / 3,
            ));
            let position = tauri::PhysicalPosition::new(
                desired_x.clamp(min_x, max_x.max(min_x)),
                desired_y.clamp(min_y, max_y.max(min_y)),
            );
            let _ = window.set_position(position);
        }
        let _ = window.show();
        if focusable {
            let _ = window.set_focus();
        }
    }
}

fn recently_resized(last_resized_at: Option<Instant>) -> bool {
    last_resized_at
        .map(|resized_at| resized_at.elapsed() < RESIZE_FOCUS_GRACE)
        .unwrap_or(false)
}

fn trigger_position(
    anchor: &SelectionAnchor,
    width: i32,
    height: i32,
    work_x: i32,
    work_y: i32,
    work_width: i32,
    work_height: i32,
) -> (i32, i32) {
    let width = width.max(1);
    let height = height.max(1);
    let work_right = work_x.saturating_add(work_width.max(1));
    let work_bottom = work_y.saturating_add(work_height.max(1));
    let gap = 8;
    let right = anchor
        .x
        .saturating_add(anchor.width.max(0))
        .saturating_add(gap);
    let left = anchor.x.saturating_sub(width).saturating_sub(gap);
    let below = anchor
        .y
        .saturating_add(anchor.height.max(0))
        .saturating_add(gap);
    let above = anchor.y.saturating_sub(height).saturating_sub(gap);
    let candidates = [(right, below), (left, below), (right, above), (left, above)];
    if let Some(position) = candidates.into_iter().find(|(x, y)| {
        *x >= work_x
            && *y >= work_y
            && x.saturating_add(width) <= work_right
            && y.saturating_add(height) <= work_bottom
    }) {
        return position;
    }
    (
        right.clamp(work_x, work_right.saturating_sub(width).max(work_x)),
        below.clamp(work_y, work_bottom.saturating_sub(height).max(work_y)),
    )
}

fn normalize_source_text(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn longest_common_contiguous_valid_chars(left: &str, right: &str) -> usize {
    let left = left.chars().collect::<Vec<_>>();
    let right = right.chars().collect::<Vec<_>>();
    if left.is_empty() || right.is_empty() {
        return 0;
    }

    let mut previous_lengths = vec![0usize; right.len() + 1];
    let mut previous_valid = vec![false; right.len() + 1];
    let mut longest = 0;
    for left_char in left {
        let mut current_lengths = vec![0usize; right.len() + 1];
        let mut current_valid = vec![false; right.len() + 1];
        for (index, right_char) in right.iter().enumerate() {
            if left_char != *right_char {
                continue;
            }
            let end = index + 1;
            current_lengths[end] = previous_lengths[index] + 1;
            current_valid[end] = previous_valid[index] || left_char.is_alphanumeric();
            if current_valid[end] {
                longest = longest.max(current_lengths[end]);
            }
        }
        previous_lengths = current_lengths;
        previous_valid = current_valid;
    }
    longest
}

fn process_image_name(image_path: &str) -> &str {
    image_path.rsplit(['\\', '/']).next().unwrap_or_default()
}

fn is_clipboard_process_name_allowed(image_path: &str) -> bool {
    let image_name = process_image_name(image_path);
    CLIPBOARD_SOURCE_PROCESS_WHITELIST
        .iter()
        .any(|allowed| image_name.eq_ignore_ascii_case(allowed))
}

fn reconcile_source_texts(
    uia_text: Option<&str>,
    clipboard_text: Result<Option<&str>, ()>,
) -> SourceReconciliation {
    let uia_text = uia_text
        .map(normalize_source_text)
        .filter(|text| !text.is_empty());
    let clipboard_text = clipboard_text
        .ok()
        .flatten()
        .map(normalize_source_text)
        .filter(|text| !text.is_empty());

    match (uia_text.as_deref(), clipboard_text.as_deref()) {
        (Some(uia_text), Some(clipboard_text)) if uia_text == clipboard_text => {
            SourceReconciliation {
                comparison: SourceComparison::Exact,
                choice: SourceChoice::UiAutomation,
            }
        }
        (Some(uia_text), Some(clipboard_text))
            if longest_common_contiguous_valid_chars(uia_text, clipboard_text) > 0 =>
        {
            SourceReconciliation {
                comparison: SourceComparison::PartialOverlap,
                choice: SourceChoice::Clipboard,
            }
        }
        // The caller reaches this function only after the source process has
        // passed the Zotero clipboard whitelist and the user explicitly
        // requested a read. A successful protected copy is therefore a
        // stronger range signal than a UIA string known to suffer offsets.
        (Some(_), Some(_)) => SourceReconciliation {
            comparison: SourceComparison::NoOverlap,
            choice: SourceChoice::Clipboard,
        },
        (Some(_), None) => SourceReconciliation {
            comparison: SourceComparison::ClipboardUnavailable,
            choice: SourceChoice::UiAutomation,
        },
        (None, Some(_)) => SourceReconciliation {
            comparison: SourceComparison::UiAutomationUnavailable,
            choice: SourceChoice::Clipboard,
        },
        (None, None) => SourceReconciliation {
            comparison: SourceComparison::BothUnavailable,
            choice: SourceChoice::Unavailable,
        },
    }
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
        types::{Point, TreeScope},
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
            Some("UI Automation 当前不可用，将仅尝试受支持的只读来源".to_string())
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
                    let generation = service.current_generation();
                    let context = source_context_at_point(
                        generation,
                        x,
                        y,
                        Some((x, y)),
                        None,
                        Instant::now(),
                    )?;
                    if context.process_id == std::process::id() {
                        return None;
                    }
                    service.invalidate_for_new_selection();
                    prepared = None;
                    pending_prepared = None;
                    Some(MouseSelectionOperation {
                        context: SelectionSourceContext {
                            generation: service.current_generation(),
                            ..context
                        },
                    })
                });
            } else if !button_down && left_button_down {
                left_button_down = false;
                if let Some(operation) = mouse_operation.take() {
                    let Some((x, y)) = cursor_position() else {
                        continue;
                    };
                    let Some(release_context) = source_context_at_point(
                        service.current_generation(),
                        x,
                        y,
                        operation.context.press_point,
                        Some((x, y)),
                        operation.context.started_at,
                    ) else {
                        continue;
                    };
                    if same_source_window(&operation.context, &release_context)
                        && point_in_root_window(release_context.root_window, x, y)
                        && is_real_drag(
                            operation.context.press_point,
                            release_context.release_point,
                        )
                        && service.owns_generation(operation.context.generation)
                    {
                        let anchor = SelectionAnchor {
                            x,
                            y,
                            width: 0,
                            height: 0,
                        };
                        let mut context = operation.context;
                        context.release_point = release_context.release_point;
                        pending_mouse_release = Some(MouseSelectionRelease {
                            context,
                            anchor,
                            ready_at: now + MOUSE_RELEASE_DELAY,
                            probe_until: now + MOUSE_RELEASE_DELAY + FOCUS_POLL_INTERVAL * 5,
                            release_probe_done: false,
                        });
                        next_focus_poll = now + MOUSE_RELEASE_DELAY;
                    }
                }
            }

            let release_probe = pending_mouse_release
                .as_ref()
                .filter(|release| now >= release.ready_at && !release.release_probe_done)
                .map(|release| (release.context, release.anchor.clone()));
            if let Some((context, anchor)) = release_probe {
                if let Some(release) = pending_mouse_release.as_mut() {
                    release.release_probe_done = true;
                }
                let value = automation
                    .as_ref()
                    .zip(walker.as_ref())
                    .and_then(|(automation, walker)| {
                        let (x, y) = context.release_point?;
                        automation
                            .element_from_point(Point::new(x, y))
                            .ok()
                            .and_then(|element| {
                                prepare_element(
                                    &service,
                                    &element,
                                    walker,
                                    PrepareElementRequest {
                                        trigger: SelectionTrigger::Automatic,
                                        trigger_id: None,
                                        fallback_anchor: Some(anchor.clone()),
                                        source_context: Some(context),
                                        allow_clipboard_fallback: true,
                                    },
                                )
                            })
                    })
                    .or_else(|| {
                        prepare_standard_edit(
                            &service,
                            context,
                            SelectionTrigger::Automatic,
                            None,
                            Some(anchor.clone()),
                        )
                    });
                if let Some(value) = value {
                    accept_automatic_prepared(
                        &service,
                        value,
                        &mut pending_prepared,
                        &mut prepared,
                    );
                    pending_mouse_release = None;
                }
            }

            let focus_probe = pending_mouse_release
                .as_ref()
                .filter(|release| {
                    now >= release.ready_at
                        && now <= release.probe_until
                        && now >= next_focus_poll
                        && !service.has_automatic_trigger()
                })
                .filter(|release| {
                    !pending_prepared
                        .as_ref()
                        .map(|(value, _)| value.generation == release.context.generation)
                        .unwrap_or(false)
                        && !prepared
                            .as_ref()
                            .map(|value| value.generation == release.context.generation)
                            .unwrap_or(false)
                })
                .map(|release| (release.context, release.anchor.clone()));
            if let Some((context, anchor)) = focus_probe {
                next_focus_poll = now + FOCUS_POLL_INTERVAL;
                let value = automation
                    .as_ref()
                    .zip(walker.as_ref())
                    .and_then(|(automation, walker)| {
                        automation.get_focused_element().ok().and_then(|element| {
                            prepare_element(
                                &service,
                                &element,
                                walker,
                                PrepareElementRequest {
                                    trigger: SelectionTrigger::Automatic,
                                    trigger_id: None,
                                    fallback_anchor: Some(anchor.clone()),
                                    source_context: Some(context),
                                    allow_clipboard_fallback: false,
                                },
                            )
                        })
                    })
                    .or_else(|| {
                        prepare_standard_edit(
                            &service,
                            context,
                            SelectionTrigger::Automatic,
                            None,
                            Some(anchor.clone()),
                        )
                    });
                if let Some(value) = value {
                    accept_automatic_prepared(
                        &service,
                        value,
                        &mut pending_prepared,
                        &mut prepared,
                    );
                    pending_mouse_release = None;
                }
            }
            if pending_mouse_release
                .as_ref()
                .map(|release| now > release.probe_until)
                .unwrap_or(false)
            {
                pending_mouse_release = None;
            }
        } else {
            left_button_down = false;
            mouse_operation = None;
            pending_mouse_release = None;
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
                                let Some((x, y)) = cursor_position() else {
                                    return Ok(());
                                };
                                let generation = service_for_event.current_generation();
                                let sender_process_id = sender.get_process_id().ok();
                                let anchor = SelectionAnchor {
                                    x,
                                    y,
                                    width: 0,
                                    height: 0,
                                };
                                let context = source_context_at_point(
                                    generation,
                                    x,
                                    y,
                                    None,
                                    Some((x, y)),
                                    Instant::now(),
                                )
                                .filter(|context| Some(context.process_id) == sender_process_id)
                                .or_else(|| {
                                    source_context_from_element(sender, generation, Some(&anchor))
                                })
                                .or_else(|| {
                                    foreground_source_context(generation, Some(&anchor)).filter(
                                        |context| Some(context.process_id) == sender_process_id,
                                    )
                                });
                                let Some(context) = context else {
                                    return Ok(());
                                };
                                if Some(context.process_id) != sender_process_id
                                    || !element_matches_source_context(sender, &context)
                                {
                                    return Ok(());
                                }
                                if let Some(value) = prepare_element(
                                    &service_for_event,
                                    sender,
                                    &walker_for_event,
                                    PrepareElementRequest {
                                        trigger: SelectionTrigger::Automatic,
                                        trigger_id: None,
                                        fallback_anchor: Some(anchor),
                                        source_context: Some(context),
                                        allow_clipboard_fallback: false,
                                    },
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
                let anchor = cursor_position().map(|(x, y)| SelectionAnchor {
                    x,
                    y,
                    width: 0,
                    height: 0,
                });
                let focused_element = automation
                    .as_ref()
                    .and_then(|automation| automation.get_focused_element().ok());
                let source_context = focused_element
                    .as_ref()
                    .and_then(|element| {
                        source_context_from_element(element, generation, anchor.as_ref())
                    })
                    .or_else(|| foreground_source_context(generation, anchor.as_ref()));
                let mut prepared = focused_element.as_ref().and_then(|element| {
                    walker.as_ref().and_then(|walker| {
                        prepare_element(
                            &service,
                            element,
                            walker,
                            PrepareElementRequest {
                                trigger: SelectionTrigger::Shortcut,
                                trigger_id: Some(trigger_id.clone()),
                                fallback_anchor: anchor.clone(),
                                source_context,
                                allow_clipboard_fallback: false,
                            },
                        )
                    })
                });
                if prepared.is_none() {
                    prepared = source_context.and_then(|context| {
                        prepare_standard_edit(
                            &service,
                            context,
                            SelectionTrigger::Shortcut,
                            Some(trigger_id.clone()),
                            anchor.clone(),
                        )
                    });
                }
                let candidate_result = if let Some(prepared) = prepared.as_ref() {
                    read_prepared_selection(prepared, true)
                } else {
                    resolve_active_selection(
                        trigger_id.clone(),
                        SelectionTrigger::Shortcut,
                        anchor.clone(),
                        None,
                        source_context,
                        true,
                    )
                };
                match candidate_result {
                    Ok(candidate)
                        if service.owns_generation(generation)
                            && service.owns_current_trigger(&trigger_id) =>
                    {
                        let _ = service.publish_candidate(candidate);
                    }
                    Ok(_) => {}
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
                let Some(value) = prepared
                    .as_ref()
                    .filter(|value| value.trigger_id == trigger_id)
                    .cloned()
                else {
                    diagnostics::info(format!(
                        "selection.trigger.read.discarded trigger_id={trigger_id}"
                    ));
                    continue;
                };
                if !service.owns_current_trigger(&trigger_id)
                    || !service.owns_generation(value.generation)
                {
                    diagnostics::info(format!(
                        "selection.trigger.read.discarded trigger_id={trigger_id}"
                    ));
                    continue;
                }
                match read_prepared_selection(&value, false) {
                    Ok(candidate)
                        if service.owns_current_trigger(&trigger_id)
                            && service.owns_generation(value.generation) =>
                    {
                        prepared = None;
                        if service.publish_candidate(candidate) {
                            diagnostics::info(format!(
                                "selection.trigger.read.completed trigger_id={trigger_id}"
                            ));
                        }
                    }
                    Ok(_) => {
                        prepared = None;
                        diagnostics::info(format!(
                            "selection.trigger.read.discarded trigger_id={trigger_id}"
                        ));
                    }
                    Err(error) => {
                        service.emit_unavailable(
                            &trigger_id,
                            SelectionTrigger::Automatic,
                            error.code,
                            &error.message,
                        );
                    }
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
    if !service.owns_generation(value.generation) || service.has_automatic_trigger() {
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
    value.source_context = value.source_context.map(|mut context| {
        context.generation = value.generation;
        context
    });
    *prepared = None;
    *pending_prepared = Some((value, Instant::now() + AUTOMATIC_DEBOUNCE));
}

#[cfg(windows)]
fn prepare_standard_edit(
    service: &SelectionService,
    context: SelectionSourceContext,
    trigger: SelectionTrigger,
    trigger_id: Option<String>,
    anchor: Option<SelectionAnchor>,
) -> Option<PreparedSelection> {
    let window = edit::find_selected_control(&context, None)?;
    Some(PreparedSelection {
        trigger_id: trigger_id.unwrap_or_else(|| Uuid::new_v4().to_string()),
        generation: service.current_generation(),
        trigger,
        anchor,
        source_context: Some(context),
        standard_edit_window: Some(window),
        source: PreparedSelectionSource::StandardEdit,
    })
}

#[cfg(windows)]
fn prepare_element(
    service: &SelectionService,
    element: &uiautomation::UIElement,
    walker: &uiautomation::UITreeWalker,
    request: PrepareElementRequest,
) -> Option<PreparedSelection> {
    use uiautomation::patterns::{UITextEditPattern, UITextPattern};
    use uiautomation::types::TextPatternRangeEndpoint;

    let PrepareElementRequest {
        trigger,
        trigger_id,
        fallback_anchor,
        source_context,
        allow_clipboard_fallback,
    } = request;
    let trigger_id = trigger_id.unwrap_or_else(|| Uuid::new_v4().to_string());
    let mut current = element.clone();
    let mut text_capable = false;
    let mut standard_edit_window = None;
    let mut source_context = source_context;
    for _ in 0..=MAX_SELECTION_ANCESTORS {
        if current.get_process_id().ok() == Some(std::process::id()) {
            break;
        }
        if let Some(context) = source_context.as_ref()
            && !element_matches_source_context(&current, context)
        {
            let Ok(parent) = walker.get_parent(&current) else {
                break;
            };
            current = parent;
            continue;
        }
        let current_context = source_context.or_else(|| {
            source_context_from_element(
                &current,
                service.current_generation(),
                fallback_anchor.as_ref(),
            )
        });
        if standard_edit_window.is_none() {
            if let (Some(context), Ok(native)) =
                (current_context.as_ref(), current.get_native_window_handle())
            {
                let native: isize = native.into();
                if native != 0
                    && edit::is_standard_edit_window(native)
                    && edit::find_selected_control(context, Some(native)) == Some(native)
                {
                    standard_edit_window = Some(native);
                    source_context = Some(*context);
                }
            }
        }
        if let Ok(pattern) = current.get_pattern::<UITextPattern>() {
            text_capable = true;
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
                    let anchor = fallback_anchor.clone().or_else(|| {
                        ranges.first().and_then(|range| {
                            range
                                .get_enclosing_element()
                                .ok()
                                .and_then(|value| value.get_bounding_rectangle().ok())
                                .and_then(anchor_from_rectangle)
                        })
                    });
                    let context = source_context.or(current_context).or_else(|| {
                        source_context_from_element(
                            &current,
                            service.current_generation(),
                            anchor.as_ref(),
                        )
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
                        source_context: context,
                        standard_edit_window,
                        source: PreparedSelectionSource::UiAutomation(agile_ranges),
                    });
                }
            }
        } else if current.get_pattern::<UITextEditPattern>().is_ok() {
            text_capable = true;
        }
        let Ok(parent) = walker.get_parent(&current) else {
            break;
        };
        current = parent;
    }
    if let Some(standard_edit_window) = standard_edit_window {
        return Some(PreparedSelection {
            trigger_id,
            generation: service.current_generation(),
            trigger,
            anchor: fallback_anchor,
            source_context,
            standard_edit_window: Some(standard_edit_window),
            source: PreparedSelectionSource::StandardEdit,
        });
    }
    if trigger == SelectionTrigger::Automatic
        && allow_clipboard_fallback
        && text_capable
        && source_context
            .as_ref()
            .map(clipboard_source_allowed)
            .unwrap_or(false)
    {
        return Some(PreparedSelection {
            trigger_id,
            generation: service.current_generation(),
            trigger,
            anchor: fallback_anchor,
            source_context,
            standard_edit_window: None,
            source: PreparedSelectionSource::ClipboardOnActivate,
        });
    }
    None
}

#[cfg(windows)]
fn same_prepared_selection(left: &PreparedSelection, right: &PreparedSelection) -> bool {
    use uiautomation::patterns::UITextRange;

    let same_context = left
        .source_context
        .zip(right.source_context)
        .map(|(left, right)| same_source_window(&left, &right))
        .unwrap_or(false);
    let same_edit = left.standard_edit_window == right.standard_edit_window;
    match (&left.source, &right.source) {
        (
            PreparedSelectionSource::ClipboardOnActivate,
            PreparedSelectionSource::ClipboardOnActivate,
        ) => left.anchor == right.anchor && same_context && same_edit,
        (PreparedSelectionSource::StandardEdit, PreparedSelectionSource::StandardEdit) => {
            left.anchor == right.anchor && same_context && same_edit
        }
        (
            PreparedSelectionSource::UiAutomation(left_ranges),
            PreparedSelectionSource::UiAutomation(right_ranges),
        ) => {
            if !same_context || !same_edit {
                return false;
            }
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
fn read_prepared_uia(prepared: &PreparedSelection) -> provider::ReadOutcome {
    use uiautomation::patterns::UITextRange;

    let PreparedSelectionSource::UiAutomation(ranges) = &prepared.source else {
        return provider::ReadOutcome::Unsupported;
    };
    let mut text = String::new();
    let max_length = (provider::MAX_TEXT_UNITS as i32).saturating_add(1);
    for agile_range in ranges {
        let Ok(range) = agile_range.resolve().map(UITextRange::from) else {
            return provider::ReadOutcome::Stale;
        };
        let Ok(value) = range.get_text(max_length) else {
            return provider::ReadOutcome::Failed;
        };
        text.push_str(&value);
        if text.encode_utf16().count() > provider::MAX_TEXT_UNITS {
            return provider::ReadOutcome::Unsupported;
        }
    }
    if text.trim().is_empty() {
        provider::ReadOutcome::NoSelection
    } else {
        provider::ReadOutcome::Selected(text)
    }
}

#[cfg(windows)]
fn no_selection_failure() -> SelectionCaptureFailure {
    SelectionCaptureFailure::new("no_selection", "当前没有可读取的文本选区")
}

#[cfg(windows)]
fn failure_from_source_outcome(outcome: &provider::ReadOutcome) -> SelectionCaptureFailure {
    SelectionCaptureFailure::new(
        provider::outcome_code(outcome),
        provider::outcome_message(outcome),
    )
}

#[cfg(windows)]
fn failure_from_source_outcomes(outcomes: &[provider::ReadOutcome]) -> SelectionCaptureFailure {
    provider::preferred_failure(outcomes.iter())
        .map(failure_from_source_outcome)
        .unwrap_or_else(no_selection_failure)
}

#[cfg(windows)]
fn resolve_active_selection(
    trigger_id: String,
    trigger: SelectionTrigger,
    anchor: Option<SelectionAnchor>,
    uia_text: Option<String>,
    source_context: Option<SelectionSourceContext>,
    wait_for_shortcut_release_before_clipboard: bool,
) -> Result<SelectionCandidate, SelectionCaptureFailure> {
    let make_candidate = |source_text| SelectionCandidate {
        trigger_id: trigger_id.clone(),
        source_text,
        anchor: anchor.clone(),
        trigger,
    };

    let Some(context) = source_context else {
        return uia_text
            .map(make_candidate)
            .ok_or_else(no_selection_failure);
    };
    if !clipboard_source_allowed(&context) {
        return uia_text
            .map(make_candidate)
            .ok_or_else(no_selection_failure);
    }

    let clipboard_result = if wait_for_shortcut_release_before_clipboard {
        wait_for_shortcut_release().and_then(|_| capture_clipboard_selection(&context))
    } else {
        capture_clipboard_selection(&context)
    };
    let clipboard_text = clipboard_result
        .as_ref()
        .map(|text| (!text.trim().is_empty()).then_some(text.as_str()))
        .map_err(|_| ());
    if let Err(error) = &clipboard_result {
        diagnostics::info(format!(
            "selection.clipboard.capture.failed code={}",
            error.code
        ));
    }
    let reconciliation = reconcile_source_texts(uia_text.as_deref(), clipboard_text);
    diagnostics::info(format!(
        "selection.source.reconciled comparison={:?} choice={:?}",
        reconciliation.comparison, reconciliation.choice
    ));

    match reconciliation.choice {
        SourceChoice::UiAutomation => uia_text
            .map(make_candidate)
            .ok_or_else(no_selection_failure),
        SourceChoice::Clipboard => match clipboard_result {
            Ok(source_text) if !source_text.trim().is_empty() => Ok(make_candidate(source_text)),
            Ok(_) => Err(no_selection_failure()),
            Err(error) => Err(error),
        },
        SourceChoice::Unavailable => {
            Err(clipboard_result.err().unwrap_or_else(no_selection_failure))
        }
    }
}

#[cfg(windows)]
fn read_prepared_selection(
    prepared: &PreparedSelection,
    wait_for_shortcut_release_before_clipboard: bool,
) -> Result<SelectionCandidate, SelectionCaptureFailure> {
    let make_candidate = |source_text| SelectionCandidate {
        trigger_id: prepared.trigger_id.clone(),
        source_text,
        anchor: prepared.anchor.clone(),
        trigger: prepared.trigger,
    };
    let mut outcomes = Vec::new();

    if let (Some(context), Some(window)) = (
        prepared.source_context.as_ref(),
        prepared.standard_edit_window,
    ) {
        match edit::read_selection(context, Some(window)) {
            provider::ReadOutcome::Selected(text) => return Ok(make_candidate(text)),
            outcome => outcomes.push(outcome),
        }
    }

    let uia_text = match &prepared.source {
        PreparedSelectionSource::UiAutomation(_) => match read_prepared_uia(prepared) {
            provider::ReadOutcome::Selected(text) => Some(text),
            outcome => {
                outcomes.push(outcome);
                None
            }
        },
        PreparedSelectionSource::StandardEdit | PreparedSelectionSource::ClipboardOnActivate => {
            None
        }
    };

    if let Some(uia_text) = uia_text {
        match resolve_active_selection(
            prepared.trigger_id.clone(),
            prepared.trigger,
            prepared.anchor.clone(),
            Some(uia_text),
            prepared.source_context,
            wait_for_shortcut_release_before_clipboard,
        ) {
            Ok(candidate) => return Ok(candidate),
            Err(error) if error.code != "no_selection" => return Err(error),
            Err(_) => {}
        }
    }

    match resolve_active_selection(
        prepared.trigger_id.clone(),
        prepared.trigger,
        prepared.anchor.clone(),
        None,
        prepared.source_context,
        wait_for_shortcut_release_before_clipboard,
    ) {
        Ok(candidate) => Ok(candidate),
        Err(error) if error.code != "no_selection" => Err(error),
        Err(_) => Err(failure_from_source_outcomes(&outcomes)),
    }
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
fn root_window_from_raw(raw: isize) -> Option<isize> {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{GA_ROOT, GetAncestor, IsWindow};

    let hwnd = HWND(raw as *mut core::ffi::c_void);
    if hwnd.0.is_null() || !unsafe { IsWindow(Some(hwnd)) }.as_bool() {
        return None;
    }
    let root = unsafe { GetAncestor(hwnd, GA_ROOT) };
    if root.0.is_null() || !unsafe { IsWindow(Some(root)) }.as_bool() {
        None
    } else {
        Some(root.0 as isize)
    }
}

#[cfg(windows)]
fn window_identity_at_point(x: i32, y: i32) -> Option<(u32, isize)> {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::UI::WindowsAndMessaging::{
        GA_ROOT, GetAncestor, GetWindowThreadProcessId, IsWindow, WindowFromPoint,
    };

    let window = unsafe { WindowFromPoint(POINT { x, y }) };
    if window.0.is_null() {
        return None;
    }
    let root = unsafe { GetAncestor(window, GA_ROOT) };
    if root.0.is_null() || !unsafe { IsWindow(Some(root)) }.as_bool() {
        return None;
    }
    let mut process_id = 0;
    let thread_id = unsafe { GetWindowThreadProcessId(root, Some(&mut process_id)) };
    (thread_id != 0 && process_id != 0).then_some((process_id, root.0 as isize))
}

#[cfg(windows)]
fn source_context_at_point(
    generation: u64,
    x: i32,
    y: i32,
    press_point: Option<(i32, i32)>,
    release_point: Option<(i32, i32)>,
    started_at: Instant,
) -> Option<SelectionSourceContext> {
    let (process_id, root_window) = window_identity_at_point(x, y)?;
    Some(SelectionSourceContext {
        generation,
        process_id,
        root_window,
        press_point,
        release_point,
        started_at,
    })
}

#[cfg(windows)]
fn source_context_from_element(
    element: &uiautomation::UIElement,
    generation: u64,
    anchor: Option<&SelectionAnchor>,
) -> Option<SelectionSourceContext> {
    let native: isize = element.get_native_window_handle().ok()?.into();
    let root_window = root_window_from_raw(native)?;
    let process_id = process_id_from_root(root_window)?;
    if process_id == std::process::id() {
        return None;
    }
    let point = anchor.map(|value| (value.x, value.y));
    Some(SelectionSourceContext {
        generation,
        process_id,
        root_window,
        press_point: None,
        release_point: point,
        started_at: Instant::now(),
    })
}

#[cfg(windows)]
fn foreground_source_context(
    generation: u64,
    anchor: Option<&SelectionAnchor>,
) -> Option<SelectionSourceContext> {
    use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;

    let root_window = root_window_from_raw(unsafe { GetForegroundWindow() }.0 as isize)?;
    let process_id = process_id_from_root(root_window)?;
    if process_id == std::process::id() {
        return None;
    }
    let point = anchor.map(|value| (value.x, value.y));
    Some(SelectionSourceContext {
        generation,
        process_id,
        root_window,
        press_point: None,
        release_point: point,
        started_at: Instant::now(),
    })
}

#[cfg(windows)]
fn process_id_from_root(root_window: isize) -> Option<u32> {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{GetWindowThreadProcessId, IsWindow};

    let hwnd = HWND(root_window as *mut core::ffi::c_void);
    if hwnd.0.is_null() || !unsafe { IsWindow(Some(hwnd)) }.as_bool() {
        return None;
    }
    let mut process_id = 0;
    let thread_id = unsafe { GetWindowThreadProcessId(hwnd, Some(&mut process_id)) };
    (thread_id != 0 && process_id != 0).then_some(process_id)
}

#[cfg(windows)]
fn clipboard_source_allowed(context: &SelectionSourceContext) -> bool {
    if !CLIPBOARD_SELECTION_ENABLED {
        diagnostics::info("selection.clipboard.policy disabled_for_experiment");
        return false;
    }

    let image_path = process_image_path(context.process_id)
        .or_else(|| process_image_path_from_window(context.root_window))
        .or_else(|| process_image_name_from_snapshot(context.process_id));
    let image_name = image_path
        .as_deref()
        .map(process_image_name)
        .unwrap_or("<unknown>");
    let direct_allowed = is_clipboard_process_name_allowed(image_name);
    let inherited_allowed =
        !direct_allowed && process_tree_contains_allowed_process(context.process_id);
    let allowed = direct_allowed || inherited_allowed;
    diagnostics::info(format!(
        "selection.clipboard.policy pid={} executable={} inherited={} allowed={allowed}",
        context.process_id, image_name, inherited_allowed
    ));
    allowed
}

#[cfg(windows)]
fn process_image_path_from_window(root_window: isize) -> Option<String> {
    use windows::Win32::{Foundation::HWND, UI::WindowsAndMessaging::GetWindowModuleFileNameW};

    let window = HWND(root_window as *mut core::ffi::c_void);
    if window.0.is_null() {
        return None;
    }
    let mut buffer = vec![0u16; 32_768];
    let length = unsafe { GetWindowModuleFileNameW(window, &mut buffer) };
    let length = usize::try_from(length).ok()?;
    String::from_utf16(buffer.get(..length)?).ok()
}

#[cfg(windows)]
fn process_image_name_from_snapshot(process_id: u32) -> Option<String> {
    process_snapshot_entries()?
        .into_iter()
        .find(|entry| entry.process_id == process_id)
        .map(|entry| entry.image_name)
}

#[cfg(windows)]
fn process_tree_contains_allowed_process(process_id: u32) -> bool {
    let Some(entries) = process_snapshot_entries() else {
        return false;
    };
    let mut current_id = process_id;
    for _ in 0..64 {
        let Some(entry) = entries.iter().find(|entry| entry.process_id == current_id) else {
            return false;
        };
        if is_clipboard_process_name_allowed(&entry.image_name) {
            return true;
        }
        if entry.parent_process_id == 0 || entry.parent_process_id == current_id {
            return false;
        }
        current_id = entry.parent_process_id;
    }
    false
}

#[cfg(windows)]
fn process_snapshot_entries() -> Option<Vec<ProcessSnapshotEntry>> {
    use windows::Win32::{
        Foundation::CloseHandle,
        System::Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
            TH32CS_SNAPPROCESS,
        },
    };

    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }.ok()?;
    let mut entry = PROCESSENTRY32W {
        dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };
    let mut entries = Vec::new();
    if unsafe { Process32FirstW(snapshot, &mut entry) }.is_ok() {
        loop {
            let end = entry
                .szExeFile
                .iter()
                .position(|character| *character == 0)
                .unwrap_or(entry.szExeFile.len());
            if let Ok(image_name) = String::from_utf16(&entry.szExeFile[..end]) {
                entries.push(ProcessSnapshotEntry {
                    process_id: entry.th32ProcessID,
                    parent_process_id: entry.th32ParentProcessID,
                    image_name,
                });
            }
            if unsafe { Process32NextW(snapshot, &mut entry) }.is_err() {
                break;
            }
        }
    }
    let _ = unsafe { CloseHandle(snapshot) };
    Some(entries)
}

#[cfg(windows)]
struct ProcessSnapshotEntry {
    process_id: u32,
    parent_process_id: u32,
    image_name: String,
}

#[cfg(windows)]
fn process_image_path(process_id: u32) -> Option<String> {
    use windows::Win32::{
        Foundation::CloseHandle,
        System::Threading::{
            OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
            QueryFullProcessImageNameW,
        },
    };

    let process =
        unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, process_id) }.ok()?;
    let mut buffer = vec![0u16; 32_768];
    let mut length = buffer.len() as u32;
    let result = unsafe {
        QueryFullProcessImageNameW(
            process,
            PROCESS_NAME_WIN32,
            windows::core::PWSTR(buffer.as_mut_ptr()),
            &mut length,
        )
    };
    let _ = unsafe { CloseHandle(process) };
    result.ok()?;
    let length = usize::try_from(length).ok()?;
    String::from_utf16(buffer.get(..length)?).ok()
}

#[cfg(windows)]
fn element_matches_source_context(
    element: &uiautomation::UIElement,
    context: &SelectionSourceContext,
) -> bool {
    if element.get_process_id().ok() != Some(context.process_id) {
        return false;
    }
    let Ok(native) = element.get_native_window_handle() else {
        return true;
    };
    let native: isize = native.into();
    if native == 0 {
        return true;
    }
    root_window_from_raw(native)
        .map(|root_window| root_window == context.root_window)
        .unwrap_or(false)
}

#[cfg(windows)]
fn same_source_window(left: &SelectionSourceContext, right: &SelectionSourceContext) -> bool {
    left.process_id == right.process_id && left.root_window == right.root_window
}

#[cfg(windows)]
fn point_in_root_window(root_window: isize, x: i32, y: i32) -> bool {
    use windows::Win32::Foundation::{HWND, RECT};
    use windows::Win32::UI::WindowsAndMessaging::GetWindowRect;

    let hwnd = HWND(root_window as *mut core::ffi::c_void);
    let mut rect = RECT::default();
    if unsafe { GetWindowRect(hwnd, &mut rect) }.is_err() {
        return false;
    }
    x >= rect.left && x < rect.right && y >= rect.top && y < rect.bottom
}

fn is_real_drag(press_point: Option<(i32, i32)>, release_point: Option<(i32, i32)>) -> bool {
    let (Some((press_x, press_y)), Some((release_x, release_y))) = (press_point, release_point)
    else {
        return false;
    };
    let dx = (release_x as i64 - press_x as i64).abs();
    let dy = (release_y as i64 - press_y as i64).abs();
    dx.max(dy) >= MIN_AUTOMATIC_DRAG_DISTANCE as i64
}

#[cfg(windows)]
fn anchor_from_rectangle(rect: uiautomation::types::Rect) -> Option<SelectionAnchor> {
    let width = rect.get_width();
    let height = rect.get_height();
    (width > 0 && height > 0).then_some(SelectionAnchor {
        x: rect.get_left(),
        y: rect.get_top(),
        width,
        height,
    })
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
fn ensure_source_window_foreground(
    context: &SelectionSourceContext,
) -> Result<(), SelectionCaptureFailure> {
    use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, SetForegroundWindow};

    let foreground = unsafe { GetForegroundWindow() };
    let foreground_root = root_window_from_raw(foreground.0 as isize);
    if foreground_root == Some(context.root_window)
        && process_id_from_root(context.root_window) == Some(context.process_id)
    {
        return Ok(());
    }
    if foreground_root.and_then(process_id_from_root) == Some(std::process::id()) {
        let target =
            windows::Win32::Foundation::HWND(context.root_window as *mut core::ffi::c_void);
        if unsafe { SetForegroundWindow(target) }.as_bool() {
            thread::sleep(Duration::from_millis(15));
            let restored = unsafe { GetForegroundWindow() };
            if root_window_from_raw(restored.0 as isize) == Some(context.root_window)
                && process_id_from_root(context.root_window) == Some(context.process_id)
            {
                return Ok(());
            }
        }
        return Err(SelectionCaptureFailure::new(
            "source_window_unavailable",
            "无法恢复选区所属的源窗口前台状态",
        ));
    }
    Err(SelectionCaptureFailure::new(
        "source_window_changed",
        "选区所属的源窗口已经切换，未向当前窗口发送复制请求",
    ))
}

#[cfg(windows)]
fn capture_clipboard_selection(
    context: &SelectionSourceContext,
) -> Result<String, SelectionCaptureFailure> {
    use uiautomation::clipboards::Clipboard;
    use uiautomation::inputs::Keyboard;

    ensure_source_window_foreground(context)?;
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
        ensure_source_window_foreground(context)?;
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
    use super::{
        SelectionAnchor, SelectionService, SourceChoice, SourceComparison, SourceReconciliation,
        is_clipboard_process_name_allowed, is_real_drag, longest_common_contiguous_valid_chars,
        normalize_shortcut, normalize_source_text, recently_resized, reconcile_source_texts,
        trigger_position,
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
    fn trigger_position_prefers_the_release_side_and_stays_in_work_area() {
        let anchor = SelectionAnchor {
            x: 100,
            y: 100,
            width: 20,
            height: 10,
        };
        assert_eq!(
            trigger_position(&anchor, 48, 48, 0, 0, 800, 600),
            (128, 118)
        );

        let edge_anchor = SelectionAnchor {
            x: 790,
            y: 590,
            width: 10,
            height: 10,
        };
        assert_eq!(
            trigger_position(&edge_anchor, 48, 48, 0, 0, 800, 600),
            (734, 534)
        );
    }

    #[test]
    fn automatic_drag_requires_a_real_release_displacement() {
        assert!(!is_real_drag(None, Some((4, 4))));
        assert!(!is_real_drag(Some((10, 10)), Some((13, 13))));
        assert!(is_real_drag(Some((10, 10)), Some((14, 10))));
    }

    #[test]
    fn source_text_normalization_trims_and_collapses_unicode_whitespace() {
        assert_eq!(
            normalize_source_text("\u{2003}中文\t\n文本\u{00a0}"),
            "中文 文本"
        );
        assert_eq!(normalize_source_text(" \t\n"), "");
    }

    #[test]
    fn exact_source_match_keeps_the_uia_text() {
        assert_eq!(
            reconcile_source_texts(Some("  中文\t文本 "), Ok(Some("中文 文本")),),
            SourceReconciliation {
                comparison: SourceComparison::Exact,
                choice: SourceChoice::UiAutomation,
            }
        );
    }

    #[test]
    fn partial_contiguous_overlap_prefers_clipboard_text() {
        assert_eq!(
            reconcile_source_texts(Some("前置中文文本"), Ok(Some("中文文本后缀"))),
            SourceReconciliation {
                comparison: SourceComparison::PartialOverlap,
                choice: SourceChoice::Clipboard,
            }
        );
    }

    #[test]
    fn no_valid_overlap_prefers_whitelisted_clipboard_text() {
        assert_eq!(
            reconcile_source_texts(Some("中文文本"), Ok(Some("English words"))),
            SourceReconciliation {
                comparison: SourceComparison::NoOverlap,
                choice: SourceChoice::Clipboard,
            }
        );
        assert_eq!(
            reconcile_source_texts(Some("..."), Ok(Some("!?"))),
            SourceReconciliation {
                comparison: SourceComparison::NoOverlap,
                choice: SourceChoice::Clipboard,
            }
        );
    }

    #[test]
    fn source_reconciliation_distinguishes_unavailable_inputs() {
        assert_eq!(
            reconcile_source_texts(Some("UIA"), Err(())),
            SourceReconciliation {
                comparison: SourceComparison::ClipboardUnavailable,
                choice: SourceChoice::UiAutomation,
            }
        );
        assert_eq!(
            reconcile_source_texts(None, Ok(Some("clipboard"))),
            SourceReconciliation {
                comparison: SourceComparison::UiAutomationUnavailable,
                choice: SourceChoice::Clipboard,
            }
        );
        assert_eq!(
            reconcile_source_texts(None, Err(())),
            SourceReconciliation {
                comparison: SourceComparison::BothUnavailable,
                choice: SourceChoice::Unavailable,
            }
        );
    }

    #[test]
    fn common_substring_counts_unicode_characters_not_utf8_bytes() {
        assert_eq!(
            longest_common_contiguous_valid_chars("前中文后", "另中文段"),
            2
        );
    }

    #[test]
    fn clipboard_whitelist_matches_only_zotero_image_names() {
        assert!(is_clipboard_process_name_allowed(
            r"C:\\Program Files\\Zotero\\zotero.exe"
        ));
        assert!(is_clipboard_process_name_allowed("/opt/ZOTERO/ZOTERO.EXE"));
        assert!(!is_clipboard_process_name_allowed(
            r"C:\\Chrome\\chrome.exe"
        ));
        assert!(!is_clipboard_process_name_allowed("zotero.exe.bak"));
    }
}
