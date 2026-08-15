"""JSONL bridge between BabelDOC and Lilt's Rust Translation Core.

The worker owns PDF parsing and output. It never receives provider credentials
and never performs an HTTP request. Every translation call is represented by a
TRANSLATE_REQUEST and is completed by the matching TRANSLATE_RESPONSE.
"""

from __future__ import annotations

import json
import logging
import multiprocessing
import os
import queue
import sys
import threading
import uuid
from asyncio import CancelledError
from contextlib import redirect_stdout
from dataclasses import dataclass
from pathlib import Path
from typing import Any, BinaryIO, Callable

PROTOCOL_VERSION = 1
MAX_LINE_BYTES = 8 * 1024 * 1024
ENGINE_VERSION = "babeldoc-0.6.4"

logger = logging.getLogger("lilt.pdf_worker")


class _Counter:
    def __init__(self) -> None:
        self._value = 0
        self._lock = threading.Lock()

    @property
    def value(self) -> int:
        with self._lock:
            return self._value

    def inc(self, amount: int) -> None:
        if amount <= 0:
            return
        with self._lock:
            self._value += amount


class WorkerProtocolError(RuntimeError):
    pass


class WorkerCancelled(RuntimeError):
    pass


class WorkerTranslationError(RuntimeError):
    pass


class WorkerEngineUnavailable(RuntimeError):
    pass


class _CancellationBridge:
    """Expose request cancellation without marking success as cancelled."""

    def __init__(self, source: threading.Event) -> None:
        self._source = source
        self._local = threading.Event()

    def is_set(self) -> bool:
        return self._source.is_set() or self._local.is_set()

    def set(self) -> None:
        self._local.set()


def _save_pdf_without_subprocess(
    pdf: Any,
    output_path: str,
    _translation_config: Any,
    *,
    garbage: int = 1,
    deflate: bool = True,
    clean: bool = True,
    deflate_fonts: bool = True,
    linear: bool = False,
    timeout: int = 120,
    tag: str = "",
) -> bool:
    """Save a BabelDOC PDF without spawning its Windows cleanup process.

    BabelDOC 0.6.4 starts a multiprocessing child for this operation. The
    Lilt Worker must keep a command reader alive while Rust answers translation
    requests, and that combination can leave the child holding the working PDF
    on Windows. A direct deflated save keeps the output valid and leaves font
    subsetting disabled by the Worker configuration.
    """
    del clean, timeout, tag
    pdf.save(
        output_path,
        garbage=garbage,
        deflate=deflate,
        clean=False,
        deflate_fonts=deflate_fonts,
        linear=linear,
    )
    return False


@dataclass(frozen=True)
class BabelDocApi:
    do_translate: Callable[..., Any]
    get_translation_stage: Callable[..., Any]
    progress_monitor: type
    translation_config: type
    watermark_output_mode: Any


def _load_babeldoc_api() -> BabelDocApi:
    """Load BabelDOC's native modules on the Worker main thread.

    BabelDOC imports cv2 and numpy through its PDF layout stack. On Windows,
    importing those native extensions for the first time from a background
    thread can block indefinitely. The synchronous entrypoint stays on the
    Worker main thread so BabelDOC can start its own Windows child processes.
    """
    # BabelDOC's fallback line clustering calls scikit-learn, whose Windows
    # CPU probe starts PowerShell. Keep that probe out of the job thread and
    # avoid an unnecessary subprocess on every PDF job.
    os.environ.setdefault("LOKY_MAX_CPU_COUNT", "1")
    try:
        from babeldoc.format.pdf.high_level import do_translate
        from babeldoc.format.pdf.high_level import get_translation_stage
        from babeldoc.format.pdf.document_il.backend.pdf_creater import PDFCreater
        from babeldoc.format.pdf.translation_config import TranslationConfig
        from babeldoc.format.pdf.translation_config import WatermarkOutputMode
        from babeldoc.progress_monitor import ProgressMonitor
    except Exception as exc:  # noqa: BLE001 - dependency boundary
        detail = str(exc).splitlines()[0][:500] or type(exc).__name__
        raise WorkerEngineUnavailable(
            f"无法加载 BabelDOC v0.6.4 运行依赖：{detail}"
        ) from exc
    if os.name == "nt":
        PDFCreater.save_pdf_with_timeout = staticmethod(_save_pdf_without_subprocess)
    return BabelDocApi(
        do_translate=do_translate,
        get_translation_stage=get_translation_stage,
        progress_monitor=ProgressMonitor,
        translation_config=TranslationConfig,
        watermark_output_mode=WatermarkOutputMode,
    )


def encode_message(message: dict[str, Any]) -> bytes:
    if not isinstance(message, dict) or not isinstance(message.get("type"), str):
        raise WorkerProtocolError("消息必须包含字符串 type")
    data = json.dumps(message, ensure_ascii=False, separators=(",", ":")).encode("utf-8")
    if len(data) + 1 > MAX_LINE_BYTES:
        raise WorkerProtocolError(f"消息超过大小限制：{len(data) + 1} 字节")
    return data + b"\n"


def decode_message(line: bytes) -> dict[str, Any]:
    if len(line) > MAX_LINE_BYTES:
        raise WorkerProtocolError(f"消息超过大小限制：{len(line)} 字节")
    line = line.rstrip(b"\r\n")
    if not line:
        raise WorkerProtocolError("消息为空")
    if b"\n" in line or b"\r" in line:
        raise WorkerProtocolError("单个读取结果不能包含多个 JSONL 帧")
    try:
        message = json.loads(line.decode("utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError) as exc:
        raise WorkerProtocolError(f"JSONL 消息解析失败：{exc}") from exc
    if not isinstance(message, dict) or not isinstance(message.get("type"), str):
        raise WorkerProtocolError("消息必须是包含字符串 type 的对象")
    return message


@dataclass(frozen=True)
class TranslationResponse:
    outcome: str
    translated_text: str | None
    translated_segments: list[dict[str, str]]
    token_usage: dict[str, int]
    cache_hit: bool
    error: dict[str, Any] | None


class ResponseRouter:
    def __init__(self, emit: Callable[[dict[str, Any]], None], cancel_event: threading.Event):
        self._emit = emit
        self._cancel_event = cancel_event
        self._lock = threading.Lock()
        self._pending: dict[str, queue.Queue[TranslationResponse]] = {}

    def request(
        self,
        *,
        task_id: str,
        mode: str,
        source_language: str,
        target_language: str,
        segments: list[dict[str, Any]],
        document_context: dict[str, Any] | None = None,
        engine_constraints: dict[str, Any] | None = None,
    ) -> TranslationResponse:
        request_id = str(uuid.uuid4())
        response_queue: queue.Queue[TranslationResponse] = queue.Queue(maxsize=1)
        with self._lock:
            self._pending[request_id] = response_queue
        try:
            self._emit(
                {
                    "type": "TRANSLATE_REQUEST",
                    "task_id": task_id,
                    "translation_request_id": request_id,
                    "mode": mode,
                    "source_language": source_language,
                    "target_language": target_language,
                    "segments": segments,
                    "document_context": document_context or {},
                    "engine_constraints": engine_constraints or {},
                }
            )
            while True:
                if self._cancel_event.is_set():
                    raise WorkerCancelled("用户取消了 PDF 翻译")
                try:
                    response = response_queue.get(timeout=0.1)
                    if response.outcome == "completed":
                        return response
                    if response.outcome == "cancelled":
                        raise WorkerCancelled("Rust Translation Core 已取消翻译")
                    error = response.error or {}
                    raise WorkerTranslationError(str(error.get("message") or "翻译请求失败"))
                except queue.Empty:
                    continue
        finally:
            with self._lock:
                self._pending.pop(request_id, None)

    def resolve(self, message: dict[str, Any]) -> None:
        request_id = message.get("translation_request_id")
        if not isinstance(request_id, str) or not request_id:
            raise WorkerProtocolError("TRANSLATE_RESPONSE 缺少 translation_request_id")
        with self._lock:
            response_queue = self._pending.get(request_id)
        if response_queue is None:
            raise WorkerProtocolError(f"未知或重复的 translation_request_id：{request_id}")
        response_queue.put(
            TranslationResponse(
                outcome=str(message.get("outcome") or "failed"),
                translated_text=message.get("translated_text")
                if isinstance(message.get("translated_text"), str)
                else None,
                translated_segments=_translated_segments(message.get("translated_segments")),
                token_usage=_token_usage(message.get("token_usage")),
                cache_hit=bool(message.get("cache_hit", False)),
                error=message.get("error") if isinstance(message.get("error"), dict) else None,
            )
        )


class LiltTranslator:
    """BabelDOC BaseTranslator-compatible adapter without a Provider client."""

    name = "lilt"

    def __init__(
        self,
        *,
        task_id: str,
        lang_in: str,
        lang_out: str,
        router: ResponseRouter,
        cancel_event: threading.Event,
    ) -> None:
        self.lang_in = lang_in
        self.lang_out = lang_out
        self.task_id = task_id
        self.router = router
        self.cancel_event = cancel_event
        self.model = "lilt-translation-core"
        self.token_count = _Counter()
        self.prompt_token_count = _Counter()
        self.completion_token_count = _Counter()
        self.cache_hit_prompt_token_count = _Counter()

    def translate(self, text: str, ignore_cache: bool = False, rate_limit_params: dict | None = None) -> str:
        return self._translate(text, rate_limit_params=rate_limit_params)

    def llm_translate(self, text: str, ignore_cache: bool = False, rate_limit_params: dict | None = None) -> str:
        return self._translate(text, rate_limit_params=rate_limit_params)

    def do_translate(self, text: str, rate_limit_params: dict | None = None) -> str:
        return self._translate(text, rate_limit_params=rate_limit_params)

    def do_llm_translate(self, text: str | None, rate_limit_params: dict | None = None) -> str | None:
        if text is None:
            return None
        return self._translate(text, rate_limit_params=rate_limit_params)

    def _translate(self, text: str, *, rate_limit_params: dict | None) -> str:
        segments, batch_shape = _segments_from_engine_text(text)
        response = self.router.request(
            task_id=self.task_id,
            mode="pdf_segment",
            source_language=self.lang_in,
            target_language=self.lang_out,
            segments=segments,
            engine_constraints={
                "response_format": "json" if batch_shape else "text",
                "placeholder_policy": "preserve",
                "request_json_mode": bool((rate_limit_params or {}).get("request_json_mode")),
            },
        )
        self._record_usage(response.token_usage)
        if batch_shape:
            return _batch_response_text(response, batch_shape)
        return response.translated_text or ""

    def get_formular_placeholder(self, placeholder_id: int | str) -> tuple[str, str]:
        value = str(placeholder_id)
        return "{v" + value + "}", r"{\s*v\s*" + value + r"\s*}"

    def get_rich_text_left_placeholder(self, placeholder_id: int | str) -> tuple[str, str]:
        value = str(placeholder_id)
        return f"<style id='{value}'>", r"<\s*style\s*id\s*=\s*'\s*" + value + r"\s*'\s*>"

    def get_rich_text_right_placeholder(self, placeholder_id: int | str) -> tuple[str, str]:
        return "</style>", r"<\s*\/\s*style\s*>"

    def _record_usage(self, usage: dict[str, int]) -> None:
        self.token_count.inc(usage.get("total_tokens", 0))
        self.prompt_token_count.inc(usage.get("prompt_tokens", 0))
        self.completion_token_count.inc(usage.get("completion_tokens", 0))
        self.cache_hit_prompt_token_count.inc(usage.get("cache_hit_prompt_tokens", 0))


class BabelDocWorker:
    def __init__(self, *, emit: Callable[[dict[str, Any]], None]) -> None:
        self.emit = emit
        self.cancel_event = threading.Event()
        self.router = ResponseRouter(emit, self.cancel_event)
        self._job_thread: threading.Thread | None = None
        self._task_id: str | None = None

    def handle(self, message: dict[str, Any]) -> None:
        message_type = message["type"]
        if message_type == "START_JOB":
            self._start_job(message)
            return
        if message_type == "CANCEL_JOB":
            self._cancel_job(message)
            return
        if message_type == "TRANSLATE_RESPONSE":
            self.router.resolve(message)
            return
        raise WorkerProtocolError(f"未知的 Rust → Worker 消息类型：{message_type}")

    def _start_job(
        self,
        message: dict[str, Any],
        *,
        run_in_thread: bool = True,
        babeldoc_api: BabelDocApi | None = None,
    ) -> None:
        if self._job_thread is not None and self._job_thread.is_alive():
            raise WorkerProtocolError("当前 Worker 已经有运行中的任务")
        if message.get("protocol_version") != PROTOCOL_VERSION:
            raise WorkerProtocolError(f"协议版本不受支持：{message.get('protocol_version')}")
        if message.get("engine_version") != ENGINE_VERSION:
            raise WorkerProtocolError(f"PDF Engine 版本不受支持：{message.get('engine_version')}")
        task_id = _required_string(message, "task_id")
        input_pdf = Path(_required_string(message, "input_pdf"))
        output_dir = Path(_required_string(message, "output_dir"))
        if not input_pdf.is_file():
            raise WorkerProtocolError("输入 PDF 不存在")
        output_dir.mkdir(parents=True, exist_ok=True)
        self._task_id = task_id
        self.cancel_event.clear()
        self.emit(
            {
                "type": "JOB_STARTED",
                "protocol_version": PROTOCOL_VERSION,
                "task_id": task_id,
                "worker_version": ENGINE_VERSION,
            }
        )
        self.emit({"type": "STAGE_CHANGED", "task_id": task_id, "stage": "engine_starting"})
        if babeldoc_api is None:
            try:
                with redirect_stdout(sys.stderr):
                    babeldoc_api = _load_babeldoc_api()
            except WorkerEngineUnavailable as exc:
                self.emit(
                    {
                        "type": "ERROR",
                        "task_id": task_id,
                        "error": {
                            "code": "engine_unavailable",
                            "message": str(exc),
                            "retryable": False,
                        },
                    }
                )
                return
        if not run_in_thread:
            self._run_job(message, input_pdf, output_dir, babeldoc_api)
            return
        self._job_thread = threading.Thread(
            target=self._run_job,
            args=(message, input_pdf, output_dir, babeldoc_api),
            name=f"lilt-pdf-job-{task_id}",
            daemon=True,
        )
        self._job_thread.start()

    def _cancel_job(self, message: dict[str, Any]) -> None:
        if message.get("task_id") != self._task_id:
            raise WorkerProtocolError("CANCEL_JOB 的 task_id 与当前任务不匹配")
        self.cancel_event.set()

    def _run_job(
        self,
        message: dict[str, Any],
        input_pdf: Path,
        output_dir: Path,
        babeldoc_api: BabelDocApi,
    ) -> None:
        task_id = self._task_id
        assert task_id is not None
        try:
            with redirect_stdout(sys.stderr):
                result = _run_babeldoc(
                    message,
                    input_pdf,
                    output_dir,
                    self.router,
                    self.cancel_event,
                    self.emit,
                    babeldoc_api,
                )
            if self.cancel_event.is_set():
                self.emit({"type": "CANCELLED", "task_id": task_id, "reason": "user_requested"})
                return
            self.emit(
                {
                    "type": "FINISHED",
                    "task_id": task_id,
                    "output_pdf": result["output_pdf"],
                    "output_mode": result.get("output_mode"),
                    "page_count": result.get("page_count"),
                    "warnings": result.get("warnings", []),
                }
            )
        except WorkerCancelled as exc:
            self.emit({"type": "CANCELLED", "task_id": task_id, "reason": str(exc)})
        except Exception as exc:  # noqa: BLE001 - boundary converts all engine errors
            logger.exception("PDF Worker job failed")
            self.emit(
                {
                    "type": "ERROR",
                    "task_id": task_id,
                    "error": {
                        "code": "babeldoc_failed",
                        "message": str(exc).splitlines()[0][:500] or "PDF 翻译失败",
                        "retryable": False,
                    },
                }
            )


def run_worker(stdin: BinaryIO, stdout: BinaryIO) -> None:
    write_lock = threading.Lock()

    def emit(message: dict[str, Any]) -> None:
        encoded = encode_message(message)
        with write_lock:
            stdout.write(encoded)
            stdout.flush()

    worker = BabelDocWorker(emit=emit)
    start_queue: queue.Queue[dict[str, Any] | None] = queue.Queue()
    fatal_protocol_error = threading.Event()
    try:
        with redirect_stdout(sys.stderr):
            preloaded_babeldoc_api = _load_babeldoc_api()
    except WorkerEngineUnavailable:
        preloaded_babeldoc_api = None

    def emit_protocol_error(exc: WorkerProtocolError) -> None:
        task_id = worker._task_id or "unknown"
        emit(
            {
                "type": "ERROR",
                "task_id": task_id,
                "error": {"code": "protocol_error", "message": str(exc), "retryable": False},
            }
        )
        if task_id == "unknown":
            fatal_protocol_error.set()
            worker.cancel_event.set()

    def read_commands() -> None:
        try:
            for line in stdin:
                try:
                    message = decode_message(line)
                    if message["type"] == "START_JOB":
                        start_queue.put(message)
                    else:
                        worker.handle(message)
                except WorkerProtocolError as exc:
                    emit_protocol_error(exc)
                    if fatal_protocol_error.is_set():
                        break
        finally:
            worker.cancel_event.set()
            start_queue.put(None)

    reader = threading.Thread(
        target=read_commands,
        name="lilt-pdf-command-reader",
        daemon=True,
    )
    reader.start()

    try:
        while not fatal_protocol_error.is_set():
            message = start_queue.get()
            if message is None:
                break
            try:
                # BabelDOC's synchronous entrypoint must run on the Worker main
                # thread because its PDF writer starts Windows child processes.
                worker._start_job(
                    message,
                    run_in_thread=False,
                    babeldoc_api=preloaded_babeldoc_api,
                )
            except WorkerProtocolError as exc:
                emit_protocol_error(exc)
    finally:
        worker.cancel_event.set()
        if worker._job_thread is not None:
            worker._job_thread.join(timeout=2)
        reader.join(timeout=2)


def _run_babeldoc(
    start: dict[str, Any],
    input_pdf: Path,
    output_dir: Path,
    router: ResponseRouter,
    cancel_event: threading.Event,
    emit: Callable[[dict[str, Any]], None],
    babeldoc_api: BabelDocApi,
) -> dict[str, Any]:
    """Create the fixed BabelDOC config and run its synchronous entrypoint."""
    do_translate = babeldoc_api.do_translate
    get_translation_stage = babeldoc_api.get_translation_stage
    ProgressMonitor = babeldoc_api.progress_monitor
    TranslationConfig = babeldoc_api.translation_config
    WatermarkOutputMode = babeldoc_api.watermark_output_mode

    options = start.get("pdf_options")
    if not isinstance(options, dict):
        options = {}
    lang_in = str(options.get("source_language") or "en")
    lang_out = str(options.get("target_language") or "zh-CN")
    translator = LiltTranslator(
        task_id=str(start["task_id"]),
        lang_in=lang_in,
        lang_out=lang_out,
        router=router,
        cancel_event=cancel_event,
    )
    config = TranslationConfig(
        input_file=input_pdf,
        output_dir=output_dir,
        working_dir=output_dir / "working",
        translator=translator,
        lang_in=lang_in,
        lang_out=lang_out,
        doc_layout_model=None,
        pages=options.get("pages") if isinstance(options.get("pages"), str) else None,
        no_dual=bool(options.get("no_dual", False)),
        no_mono=bool(options.get("no_mono", False)),
        qps=1,
        debug=False,
        skip_clean=os.name == "nt",
        use_alternating_pages_dual=bool(options.get("alternating_pages", False)),
        watermark_output_mode=WatermarkOutputMode.NoWatermark,
        custom_system_prompt=None,
        glossaries=None,
        auto_extract_glossary=False,
    )
    engine_cancel_event = _CancellationBridge(cancel_event)

    def report_progress(**event: Any) -> None:
        progress: dict[str, Any] = {
            "type": "PROGRESS",
            "task_id": str(start["task_id"]),
            "stage": str(event.get("stage") or event.get("type") or "engine"),
        }
        current = event.get("stage_current")
        total = event.get("stage_total")
        fraction = event.get("overall_progress")
        if isinstance(current, int | float):
            progress["current"] = int(current)
        if isinstance(total, int | float):
            progress["total"] = int(total)
        if isinstance(fraction, int | float):
            progress["fraction"] = max(0.0, min(1.0, float(fraction) / 100.0))
        if isinstance(event.get("message"), str):
            progress["message"] = event["message"]
        emit(progress)

    progress_monitor = ProgressMonitor(
        get_translation_stage(config),
        progress_change_callback=report_progress,
        finish_callback=lambda **_event: None,
        cancel_event=engine_cancel_event,
        report_interval=0.1,
    )
    config.progress_monitor = progress_monitor
    try:
        result = do_translate(progress_monitor, config)
    except CancelledError as exc:
        raise WorkerCancelled("用户取消了 PDF 翻译") from exc
    if cancel_event.is_set():
        raise WorkerCancelled("用户取消了 PDF 翻译")
    return _translate_result_to_payload(result, options)


def _translate_result_to_payload(result: Any, options: dict[str, Any]) -> dict[str, Any]:
    if result is None:
        raise RuntimeError("BabelDOC 完成事件缺少结果")
    output_mode = str(options.get("output_mode") or "bilingual")
    candidates = (
        ("mono_pdf_path", "mono"),
        ("dual_pdf_path", "bilingual"),
        ("no_watermark_mono_pdf_path", "mono"),
        ("no_watermark_dual_pdf_path", "bilingual"),
    )
    preferred = ["dual_pdf_path", "no_watermark_dual_pdf_path"]
    if output_mode == "mono":
        preferred = ["mono_pdf_path", "no_watermark_mono_pdf_path"]
    output_pdf = next((getattr(result, name, None) for name in preferred if getattr(result, name, None)), None)
    if output_pdf is None:
        output_pdf = next((getattr(result, name, None) for name, _ in candidates if getattr(result, name, None)), None)
    if output_pdf is None:
        raise RuntimeError("BabelDOC 未生成输出 PDF")
    return {
        "output_pdf": str(Path(output_pdf).resolve()),
        "output_mode": output_mode,
        "page_count": None,
        "warnings": [],
    }


def _segments_from_engine_text(text: str) -> tuple[list[dict[str, Any]], list[str] | None]:
    batch_marker = "## Here is the input:"
    if batch_marker in text:
        text = text.rsplit(batch_marker, 1)[1].strip()
    else:
        single_marker = "Now translate the following text:"
        if single_marker in text:
            text = text.rsplit(single_marker, 1)[1].strip()
    try:
        parsed = json.loads(text)
    except (TypeError, json.JSONDecodeError):
        parsed = None
    if isinstance(parsed, list) and all(isinstance(item, dict) for item in parsed):
        segments: list[dict[str, Any]] = []
        ids: list[str] = []
        for index, item in enumerate(parsed):
            segment_id = str(item.get("id") or index)
            source_text = item.get("input")
            if not isinstance(source_text, str):
                return [{"segment_id": "segment-0", "source_text": text, "placeholders": []}], None
            ids.append(segment_id)
            segments.append(
                {
                    "segment_id": segment_id,
                    "source_text": source_text,
                    "placeholders": item.get("placeholders", []) if isinstance(item.get("placeholders"), list) else [],
                }
            )
        return segments, ids
    return [{"segment_id": "segment-0", "source_text": text, "placeholders": []}], None


def _batch_response_text(response: TranslationResponse, ids: list[str]) -> str:
    translations = {item["segment_id"]: item["translated_text"] for item in response.translated_segments}
    if not translations and response.translated_text:
        try:
            parsed = json.loads(response.translated_text)
            if isinstance(parsed, list):
                return json.dumps(parsed, ensure_ascii=False)
        except json.JSONDecodeError:
            pass
    output = [{"id": segment_id, "output": translations.get(segment_id, "")} for segment_id in ids]
    return json.dumps(output, ensure_ascii=False)


def _translated_segments(value: Any) -> list[dict[str, str]]:
    if not isinstance(value, list):
        return []
    return [
        {"segment_id": str(item["segment_id"]), "translated_text": str(item["translated_text"])}
        for item in value
        if isinstance(item, dict) and "segment_id" in item and "translated_text" in item
    ]


def _token_usage(value: Any) -> dict[str, int]:
    if not isinstance(value, dict):
        return {}
    result: dict[str, int] = {}
    for key in ("prompt_tokens", "completion_tokens", "total_tokens", "cache_hit_prompt_tokens"):
        item = value.get(key)
        if isinstance(item, int) and item >= 0:
            result[key] = item
    return result


def _required_string(message: dict[str, Any], key: str) -> str:
    value = message.get(key)
    if not isinstance(value, str) or not value:
        raise WorkerProtocolError(f"{key} 必须是非空字符串")
    return value


if __name__ == "__main__":
    multiprocessing.freeze_support()
    logging.basicConfig(level=logging.INFO, stream=sys.stderr)
    run_worker(sys.stdin.buffer, sys.stdout.buffer)
