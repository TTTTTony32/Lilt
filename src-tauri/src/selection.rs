use crate::{
    contracts::{
        SelectionAnchor, SelectionMode, SelectionNotice, SelectionRequestPayload,
        SelectionRuntimeStatus, SelectionStatusChanged, SelectionTrigger, SelectionUnavailable,
        DEFAULT_SELECTION_MODE, DEFAULT_SELECTION_SHORTCUT,
    },
    diagnostics,
};
use std::{
    sync::{mpsc, Arc, Mutex},
    thread,
    time::{Duration, Instant},
};
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

const SELECTION_EVENT: &str = "selection_available";
const SELECTION_UNAVAILABLE_EVENT: &str = "selection_unavailable";
const SELECTION_STATUS_EVENT: &str = "selection_status_changed";
const SELECTION_OPEN_MAIN_EVENT: &str = "selection_open_main";
const SELECTION_TTL: Duration = Duration::from_secs(120);
const AUTOMATIC_DEBOUNCE: Duration = Duration::from_millis(500);

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
    latest: Option<StoredSelection>,
    pending_notice: Option<SelectionNotice>,
    window_ready: bool,
    last_signature: Option<SelectionSignature>,
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
    source_text: String,
    anchor: Option<SelectionAnchor>,
    trigger: SelectionTrigger,
}

enum WorkerCommand {
    SetAutomatic(bool),
    ReadFocused,
    Candidate(SelectionCandidate),
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
                latest: None,
                pending_notice: None,
                window_ready: false,
                last_signature: None,
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
        self.send_worker(WorkerCommand::ReadFocused);
    }

    pub fn window_ready(&self) -> Option<SelectionNotice> {
        let pending = {
            let mut inner = self.inner.lock().ok()?;
            inner.window_ready = true;
            inner.pending_notice.take()
        };
        if let Some(notice) = pending.as_ref() {
            self.show_window(notice.anchor.as_ref());
        }
        pending
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
        let active_id = {
            let mut inner = match self.inner.lock() {
                Ok(value) => value,
                Err(_) => return,
            };
            if request_id.is_none()
                || inner
                    .latest
                    .as_ref()
                    .map(|item| item.payload.request_id.as_str())
                    == request_id
            {
                inner.latest.take().map(|item| item.payload.request_id)
            } else {
                None
            }
        };
        if let Some(request_id) = active_id {
            if let Ok(mut cancellations) = self.cancellations.lock() {
                if let Some(token) = cancellations.remove(&request_id) {
                    token.cancel();
                }
            }
        }
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

    pub fn shutdown(&self) {
        let sender = self.worker.lock().ok().and_then(|mut worker| worker.take());
        if let Some(sender) = sender {
            let _ = sender.send(WorkerCommand::Shutdown);
        }
    }

    fn send_worker(&self, command: WorkerCommand) {
        if let Ok(worker) = self.worker.lock() {
            if let Some(sender) = worker.as_ref() {
                let _ = sender.send(command);
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

    fn emit_unavailable(&self, trigger: SelectionTrigger, code: &str, message: &str) {
        diagnostics::warn(format!(
            "selection.capture.failed trigger={trigger:?} code={code}"
        ));
        if let Some(app) = self.app_handle() {
            let _ = app.emit(
                SELECTION_UNAVAILABLE_EVENT,
                SelectionUnavailable {
                    request_id: None,
                    trigger,
                    code: code.to_string(),
                    message: message.to_string(),
                },
            );
        }
    }

    fn enqueue_candidate(&self, candidate: SelectionCandidate) {
        self.send_worker(WorkerCommand::Candidate(candidate));
    }

    fn publish_candidate(&self, candidate: SelectionCandidate) {
        let source_text = candidate.source_text.trim().to_string();
        if source_text.is_empty() {
            return;
        }
        let source_chars = source_text.chars().count();
        let mut inner = match self.inner.lock() {
            Ok(value) => value,
            Err(_) => return,
        };
        let signature = SelectionSignature {
            source_text: source_text.clone(),
            anchor: candidate.anchor.clone(),
        };
        if inner.last_signature.as_ref() == Some(&signature) {
            return;
        }
        inner.last_signature = Some(signature);
        let request_id = Uuid::new_v4().to_string();
        if let Ok(mut cancellations) = self.cancellations.lock() {
            if let Some(old_id) = inner
                .latest
                .as_ref()
                .map(|item| item.payload.request_id.clone())
            {
                if let Some(token) = cancellations.remove(&old_id) {
                    token.cancel();
                }
            }
        }
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
            trigger: payload.trigger,
            anchor: payload.anchor.clone(),
        };
        inner.latest = Some(StoredSelection {
            payload,
            created_at: Instant::now(),
        });
        let window_ready = inner.window_ready;
        if window_ready {
            inner.pending_notice = None;
        } else {
            inner.pending_notice = Some(notice.clone());
        }
        drop(inner);

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
            self.ensure_window(candidate.trigger);
        }
    }

    fn ensure_window(&self, trigger: SelectionTrigger) {
        let Some(app) = self.app_handle() else { return };
        if app.get_webview_window("selection").is_some() {
            return;
        }
        let result =
            WebviewWindowBuilder::new(&app, "selection", WebviewUrl::App("index.html".into()))
                .title("Lilt")
                .inner_size(440.0, 320.0)
                .min_inner_size(300.0, 180.0)
                .decorations(false)
                .resizable(false)
                .always_on_top(true)
                .skip_taskbar(true)
                .focused(false)
                .visible(false)
                .build();
        if let Err(error) = result {
            self.emit_unavailable(
                trigger,
                "selection_window_unavailable",
                &format!("无法创建划词浮窗：{error}"),
            );
        }
    }

    fn show_window(&self, anchor: Option<&SelectionAnchor>) {
        let Some(app) = self.app_handle() else { return };
        let Some(window) = app.get_webview_window("selection") else {
            return;
        };
        let size = window
            .outer_size()
            .unwrap_or(tauri::PhysicalSize::new(440, 320));
        let desired = anchor.map(|value| (value.x, value.y + value.height + 8));
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
            if matches!(command, WorkerCommand::Shutdown) {
                break;
            }
        }
    }
}

#[cfg(windows)]
fn windows_worker_loop(receiver: mpsc::Receiver<WorkerCommand>, service: SelectionService) {
    use uiautomation::{
        events::{CustomEventHandlerFn, UIEventHandler, UIEventType},
        types::TreeScope,
        UIAutomation,
    };

    let automation = match UIAutomation::new() {
        Ok(value) => {
            service.set_uia_status(true, None);
            value
        }
        Err(error) => {
            service.set_uia_status(false, Some(format!("UI Automation 初始化失败：{error}")));
            while let Ok(command) = receiver.recv() {
                if matches!(command, WorkerCommand::Shutdown) {
                    break;
                }
            }
            return;
        }
    };
    let root = match automation.get_root_element() {
        Ok(value) => value,
        Err(error) => {
            service.set_uia_status(false, Some(format!("UI Automation 根元素不可用：{error}")));
            return;
        }
    };
    let mut handler: Option<UIEventHandler> = None;
    let mut automatic = false;
    let mut pending: Option<(SelectionCandidate, Instant)> = None;

    loop {
        if let Some((candidate, deadline)) = pending.take() {
            if deadline <= Instant::now() {
                service.publish_candidate(candidate);
            } else {
                pending = Some((candidate, deadline));
            }
        }
        match receiver.recv_timeout(Duration::from_millis(40)) {
            Ok(WorkerCommand::SetAutomatic(enabled)) => {
                if enabled == automatic {
                    continue;
                }
                if enabled {
                    let service_for_event = service.clone();
                    let callback: Box<CustomEventHandlerFn> = Box::new(move |sender, _event| {
                        capture_element(&service_for_event, sender, SelectionTrigger::Automatic);
                        Ok(())
                    });
                    let event_handler = UIEventHandler::from(callback);
                    match automation.add_automation_event_handler(
                        UIEventType::Text_TextSelectionChanged,
                        &root,
                        TreeScope::Subtree,
                        None,
                        &event_handler,
                    ) {
                        Ok(()) => {
                            handler = Some(event_handler);
                            automatic = true;
                            service.set_uia_status(true, None);
                        }
                        Err(error) => service
                            .set_uia_status(false, Some(format!("自动选区监听失败：{error}"))),
                    }
                } else if let Some(event_handler) = handler.take() {
                    let _ = automation.remove_automation_event_handler(
                        UIEventType::Text_TextSelectionChanged,
                        &root,
                        &event_handler,
                    );
                    automatic = false;
                    service.set_uia_status(true, None);
                }
            }
            Ok(WorkerCommand::ReadFocused) => match automation.get_focused_element() {
                Ok(element) => capture_element(&service, &element, SelectionTrigger::Shortcut),
                Err(error) => service.emit_unavailable(
                    SelectionTrigger::Shortcut,
                    "uia_unavailable",
                    &format!("无法读取当前应用的选区：{error}"),
                ),
            },
            Ok(WorkerCommand::Candidate(candidate)) => {
                if candidate.trigger == SelectionTrigger::Automatic {
                    pending = Some((candidate, Instant::now() + AUTOMATIC_DEBOUNCE));
                } else {
                    service.publish_candidate(candidate);
                }
            }
            Ok(WorkerCommand::Shutdown) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
    }
    if let Some(event_handler) = handler {
        let _ = automation.remove_automation_event_handler(
            UIEventType::Text_TextSelectionChanged,
            &root,
            &event_handler,
        );
    }
}

#[cfg(windows)]
fn capture_element(
    service: &SelectionService,
    element: &uiautomation::UIElement,
    trigger: SelectionTrigger,
) {
    use uiautomation::patterns::UITextPattern;
    if element.get_process_id().ok() == Some(std::process::id()) {
        return;
    }
    let pattern = match element.get_pattern::<UITextPattern>() {
        Ok(value) => value,
        Err(error) => {
            service.emit_unavailable(
                trigger,
                "unsupported_control",
                &format!("目标应用不支持读取文本选区：{error}"),
            );
            return;
        }
    };
    let ranges = match pattern.get_selection() {
        Ok(value) => value,
        Err(error) => {
            service.emit_unavailable(
                trigger,
                "no_selection",
                &format!("当前没有可读取的选区：{error}"),
            );
            return;
        }
    };
    let mut text = String::new();
    let mut anchor = None;
    for range in ranges {
        if let Ok(value) = range.get_text(-1) {
            text.push_str(&value);
        }
        if anchor.is_none() {
            anchor = range
                .get_enclosing_element()
                .ok()
                .and_then(|value| value.get_bounding_rectangle().ok())
                .map(|rect| SelectionAnchor {
                    x: rect.get_left(),
                    y: rect.get_top(),
                    width: rect.get_width(),
                    height: rect.get_height(),
                });
        }
    }
    if text.trim().is_empty() {
        return;
    }
    service.enqueue_candidate(SelectionCandidate {
        source_text: text,
        anchor,
        trigger,
    });
}

#[cfg(test)]
mod tests {
    use super::normalize_shortcut;

    #[test]
    fn normalize_shortcut_accepts_common_modifier_aliases() {
        assert_eq!(normalize_shortcut(" ctrl + shift + l "), "Control+Shift+l");
        assert_eq!(normalize_shortcut("cmd+alt+k"), "Super+Alt+k");
    }
}
