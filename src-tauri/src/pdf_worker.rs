use crate::diagnostics;
use crate::pdf_protocol::{
    CancelJobMessage, MAX_PROTOCOL_LINE_BYTES, RustToWorkerMessage, WorkerToRustMessage,
    decode_worker_message, encode_rust_message,
};
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum WorkerSessionError {
    #[error("启动 PDF Worker 失败：{0}")]
    Spawn(#[source] std::io::Error),
    #[error("PDF Worker 缺少 stdin 管道")]
    MissingStdin,
    #[error("PDF Worker 缺少 stdout 管道")]
    MissingStdout,
    #[error("PDF Worker 缺少 stderr 管道")]
    MissingStderr,
    #[error("向 PDF Worker 写入消息失败：{0}")]
    Write(#[source] std::io::Error),
    #[error("PDF Worker 消息通道已关闭")]
    ChannelClosed,
    #[error("PDF Worker 会话已结束")]
    Finished,
}

#[derive(Debug)]
pub enum WorkerSessionEvent {
    Message(WorkerToRustMessage),
    ProtocolError(String),
    WorkerExited(Option<i32>),
}

/// Owns one Python Worker process.
///
/// The session deliberately exposes protocol events instead of embedding the
/// Translation Core. A Job Manager can therefore route each
/// `TRANSLATE_REQUEST` through the shared Rust core and send the response back
/// through the same serialized writer queue.
pub struct WorkerSession {
    task_id: String,
    outbound: Mutex<Option<Sender<RustToWorkerMessage>>>,
    events: Mutex<Receiver<WorkerSessionEvent>>,
    child: Arc<Mutex<Child>>,
}

impl Drop for WorkerSession {
    fn drop(&mut self) {
        if let Ok(mut child) = self.child.lock() {
            let _ = child.kill();
        }
    }
}

impl WorkerSession {
    pub fn spawn(mut command: Command, task_id: String) -> Result<Self, WorkerSessionError> {
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(WorkerSessionError::Spawn)?;
        let stdin = child.stdin.take().ok_or(WorkerSessionError::MissingStdin)?;
        let stdout = child
            .stdout
            .take()
            .ok_or(WorkerSessionError::MissingStdout)?;
        let stderr = child
            .stderr
            .take()
            .ok_or(WorkerSessionError::MissingStderr)?;

        let child = Arc::new(Mutex::new(child));
        let (outbound_tx, outbound_rx) = mpsc::channel::<RustToWorkerMessage>();
        let (events_tx, events_rx) = mpsc::channel::<WorkerSessionEvent>();

        let writer_events = events_tx.clone();
        thread::Builder::new()
            .name(format!("lilt-pdf-worker-write-{task_id}"))
            .spawn(move || write_loop(stdin, outbound_rx, writer_events))
            .map_err(WorkerSessionError::Spawn)?;

        let reader_events = events_tx.clone();
        let reader_task_id = task_id.clone();
        thread::Builder::new()
            .name(format!("lilt-pdf-worker-read-{task_id}"))
            .spawn(move || read_loop(stdout, reader_events, reader_task_id))
            .map_err(WorkerSessionError::Spawn)?;

        let stderr_task_id = task_id.clone();
        thread::Builder::new()
            .name(format!("lilt-pdf-worker-stderr-{task_id}"))
            .spawn(move || drain_stderr(stderr, stderr_task_id))
            .map_err(WorkerSessionError::Spawn)?;

        let waiter_child = child.clone();
        let waiter_events = events_tx;
        let waiter_task_id = task_id.clone();
        thread::Builder::new()
            .name(format!("lilt-pdf-worker-wait-{task_id}"))
            .spawn(move || {
                let status = waiter_child
                    .lock()
                    .ok()
                    .and_then(|mut child| child.wait().ok());
                diagnostics::info(format!(
                    "pdf.worker.exited task_id={waiter_task_id} exit_code={}",
                    status
                        .and_then(|value| value.code().map_or(Some(-1), Some))
                        .unwrap_or(-1)
                ));
                let exit_code = status.and_then(|value| value.code());
                let _ = waiter_events.send(WorkerSessionEvent::WorkerExited(exit_code));
            })
            .map_err(WorkerSessionError::Spawn)?;

        Ok(Self {
            task_id,
            outbound: Mutex::new(Some(outbound_tx)),
            events: Mutex::new(events_rx),
            child,
        })
    }

    pub fn task_id(&self) -> &str {
        &self.task_id
    }

    pub fn send(&self, message: RustToWorkerMessage) -> Result<(), WorkerSessionError> {
        let outbound = self
            .outbound
            .lock()
            .map_err(|_| WorkerSessionError::ChannelClosed)?
            .as_ref()
            .cloned()
            .ok_or(WorkerSessionError::Finished)?;
        outbound
            .send(message)
            .map_err(|_| WorkerSessionError::ChannelClosed)
    }

    pub fn cancel(&self, reason: impl Into<String>) -> Result<(), WorkerSessionError> {
        self.send(RustToWorkerMessage::CancelJob(CancelJobMessage {
            task_id: self.task_id.clone(),
            reason: reason.into(),
        }))
    }

    pub fn recv(&self) -> Result<WorkerSessionEvent, WorkerSessionError> {
        self.events
            .lock()
            .map_err(|_| WorkerSessionError::ChannelClosed)?
            .recv()
            .map_err(|_| WorkerSessionError::ChannelClosed)
    }

    pub fn recv_timeout(&self, timeout: Duration) -> Result<WorkerSessionEvent, RecvTimeoutError> {
        self.events
            .lock()
            .map_err(|_| RecvTimeoutError::Disconnected)?
            .recv_timeout(timeout)
    }

    pub fn close_writer(&self) {
        if let Ok(mut outbound) = self.outbound.lock() {
            outbound.take();
        }
    }

    pub fn terminate(&self) -> Result<(), WorkerSessionError> {
        let mut child = self
            .child
            .lock()
            .map_err(|_| WorkerSessionError::ChannelClosed)?;
        child.kill().map_err(WorkerSessionError::Write)
    }
}

fn write_loop(
    mut stdin: impl Write,
    outbound: Receiver<RustToWorkerMessage>,
    events: Sender<WorkerSessionEvent>,
) {
    for message in outbound {
        let line = match encode_rust_message(&message) {
            Ok(line) => line,
            Err(error) => {
                let _ = events.send(WorkerSessionEvent::ProtocolError(error.to_string()));
                break;
            }
        };
        if let Err(error) = stdin.write_all(line.as_bytes()).and_then(|_| stdin.flush()) {
            let _ = events.send(WorkerSessionEvent::ProtocolError(
                WorkerSessionError::Write(error).to_string(),
            ));
            break;
        }
    }
}

fn read_loop(stdout: impl std::io::Read, events: Sender<WorkerSessionEvent>, task_id: String) {
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) if line.len() > MAX_PROTOCOL_LINE_BYTES => {
                let _ = events.send(WorkerSessionEvent::ProtocolError(format!(
                    "PDF Worker 消息超过大小限制：{} 字节",
                    line.len()
                )));
                break;
            }
            Ok(_) => match decode_worker_message(&line) {
                Ok(message) => {
                    if let WorkerToRustMessage::JobStarted(start) = &message
                        && start.task_id != task_id
                    {
                        let _ = events.send(WorkerSessionEvent::ProtocolError(
                            "PDF Worker 返回了不匹配的 task_id".to_string(),
                        ));
                        break;
                    }
                    if events.send(WorkerSessionEvent::Message(message)).is_err() {
                        break;
                    }
                }
                Err(error) => {
                    let _ = events.send(WorkerSessionEvent::ProtocolError(error.to_string()));
                    break;
                }
            },
            Err(error) => {
                let _ = events.send(WorkerSessionEvent::ProtocolError(format!(
                    "读取 PDF Worker stdout 失败：{error}"
                )));
                break;
            }
        }
    }
}

fn drain_stderr(stderr: impl std::io::Read, task_id: String) {
    let reader = BufReader::new(stderr);
    for line in reader.lines() {
        if line.is_err() {
            break;
        }
        // Worker 日志可能包含来自第三方库的文档片段。这里只负责排空管道，
        // 不把原文转发到 Lilt 日志。
    }
    diagnostics::info(format!("pdf.worker.stderr.closed task_id={task_id}"));
}

#[cfg(test)]
mod tests {
    use super::{WorkerSessionEvent, read_loop};
    use crate::pdf_protocol::WorkerToRustMessage;
    use std::io::Cursor;
    use std::sync::mpsc;

    #[test]
    fn reader_routes_valid_worker_messages() {
        let input = br#"{"type":"JOB_STARTED","protocol_version":1,"task_id":"task-1"}
{"type":"STAGE_CHANGED","task_id":"task-1","stage":"parse"}
"#;
        let (events_tx, events_rx) = mpsc::channel();
        read_loop(Cursor::new(input), events_tx, "task-1".to_string());

        assert!(matches!(
            events_rx.recv().expect("job started event"),
            WorkerSessionEvent::Message(WorkerToRustMessage::JobStarted(_))
        ));
        assert!(matches!(
            events_rx.recv().expect("stage event"),
            WorkerSessionEvent::Message(WorkerToRustMessage::StageChanged(_))
        ));
    }

    #[test]
    fn reader_rejects_a_mismatched_started_task() {
        let input = br#"{"type":"JOB_STARTED","protocol_version":1,"task_id":"other-task"}
"#;
        let (events_tx, events_rx) = mpsc::channel();
        read_loop(Cursor::new(input), events_tx, "task-1".to_string());

        match events_rx.recv().expect("protocol error event") {
            WorkerSessionEvent::ProtocolError(message) => {
                assert!(message.contains("task_id"));
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }
}
