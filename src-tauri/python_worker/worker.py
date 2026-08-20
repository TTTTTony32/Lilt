"""JSONL bridge between BabelDOC and Lilt's Rust Translation Core.

The worker owns PDF parsing and output. It never receives provider credentials
and never performs an HTTP request. Every translation call is represented by a
TRANSLATE_REQUEST and is completed by the matching TRANSLATE_RESPONSE.
"""

from __future__ import annotations

import copy
import json
import logging
import multiprocessing
import os
import queue
import sys
import threading
import time
import uuid
from asyncio import CancelledError
from contextlib import redirect_stdout
from dataclasses import dataclass
from pathlib import Path
from typing import Any, BinaryIO, Callable

PROTOCOL_VERSION = 2
MAX_LINE_BYTES = 8 * 1024 * 1024
ENGINE_VERSION = "babeldoc-0.6.4"
PREFLIGHT_MAX_SAMPLES = 8
PREFLIGHT_MAX_SAMPLE_CHARS = 12_000
PREFLIGHT_MAX_CONTEXT_SEGMENTS = 3
PREFLIGHT_MAX_CONTEXT_CHARS = 4_000
PREFLIGHT_MAX_CONSTRAINT_ITEMS = 64
PREFLIGHT_TIMEOUT_SECONDS = 60.0
TASK_CONSTRAINT_MIN_CONFIDENCE = 0.6

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


@dataclass(frozen=True)
class DocumentContextState:
    """The task-local context forwarded with each PDF segment request."""

    document_context: dict[str, Any]
    context_hash: str | None
    context_before: list[Any]
    context_after: list[Any]
    task_terms: list[Any]
    abbreviations: list[Any]
    warnings: list[str]
    fallback: bool = False


@dataclass(frozen=True)
class DocumentPreflightResponse:
    outcome: str
    document_context: dict[str, Any]
    context_hash: str | None
    warnings: list[str]
    error: dict[str, Any] | None
    context_before: list[Any]
    context_after: list[Any]
    task_terms: list[Any]
    abbreviations: list[Any]
    context_valid: bool = True
    degraded: bool = False


@dataclass
class _PendingPreflightRequest:
    response_queue: queue.Queue[DocumentPreflightResponse]
    response_started: bool = False
    terminal: bool = False


class DocumentPreflightCoordinator:
    """Coordinate one optional preflight exchange for the current PDF task.

    BabelDOC owns parsing and calls the translator from its own execution
    path. The first structured segment text is therefore the safest fallback
    sample when BabelDOC has not exposed a separate document IR. The
    coordinator only sends JSON over the existing Worker pipe; it never reads
    Provider settings or calls a model itself.
    """

    def __init__(
        self,
        *,
        task_id: str,
        source_language: str,
        target_language: str,
        emit: Callable[[dict[str, Any]], None],
        cancel_event: threading.Event,
        metadata: dict[str, Any] | None = None,
        engine_constraints: dict[str, Any] | None = None,
        configured_samples: list[Any] | None = None,
        timeout_seconds: float = PREFLIGHT_TIMEOUT_SECONDS,
        on_state: Callable[[DocumentContextState], None] | None = None,
    ) -> None:
        self.task_id = task_id
        self.source_language = source_language
        self.target_language = target_language
        self._emit = emit
        self._cancel_event = cancel_event
        self._metadata = _sanitize_mapping(metadata)
        self._engine_constraints = _sanitize_mapping(engine_constraints)
        self._engine_constraints.setdefault("response_format", "json")
        self._engine_constraints.setdefault("sample_limit", PREFLIGHT_MAX_SAMPLES)
        self._engine_constraints.setdefault("sample_char_limit", PREFLIGHT_MAX_SAMPLE_CHARS)
        self._configured_samples = _limited_preflight_samples(configured_samples or [])
        self._timeout_seconds = max(0.1, min(float(timeout_seconds), 60.0))
        self._on_state = on_state
        self._pending_lock = threading.Lock()
        self._pending: dict[str, _PendingPreflightRequest] = {}
        self._ensure_lock = threading.Lock()
        self._state_lock = threading.Lock()
        self._initialized = False
        self._state = DocumentContextState({}, None, [], [], [], [], [])

    @property
    def state(self) -> DocumentContextState:
        with self._state_lock:
            return copy.deepcopy(self._state)

    def ensure(self, samples: list[dict[str, Any]]) -> DocumentContextState:
        """Run preflight once, then return the task-local state.

        A timeout, malformed response, or explicit failed outcome becomes a
        warning plus empty context. This keeps old Rust/Worker combinations
        able to complete normal PDF translation while the new protocol is
        rolled out.
        """
        with self._ensure_lock:
            if self._initialized:
                return self.state

            request_id = str(uuid.uuid4())
            response_queue: queue.Queue[DocumentPreflightResponse] = queue.Queue(maxsize=1)
            with self._pending_lock:
                self._pending[request_id] = _PendingPreflightRequest(response_queue)
            request = {
                "type": "DOCUMENT_PREFLIGHT_REQUEST",
                "task_id": self.task_id,
                "preflight_request_id": request_id,
                "source_language": self.source_language,
                "target_language": self.target_language,
                "metadata": copy.deepcopy(self._metadata),
                "samples": _limited_preflight_samples(
                    [*self._configured_samples, *samples]
                ),
                "engine_constraints": copy.deepcopy(self._engine_constraints),
            }
            try:
                self._emit(request)
            except Exception as exc:  # noqa: BLE001 - protocol boundary fallback
                with self._pending_lock:
                    self._pending.pop(request_id, None)
                return self._finish_fallback(
                    f"发送文档预检请求失败：{str(exc).splitlines()[0][:300]}",
                    preflight_request_id=request_id,
                )

            expires_at = time.monotonic() + self._timeout_seconds
            try:
                while True:
                    if self._cancel_event.is_set():
                        self._terminate(request_id)
                        raise WorkerCancelled("用户取消了 PDF 翻译")

                    with self._pending_lock:
                        pending = self._pending.get(request_id)
                        response_started = (
                            pending is not None
                            and (pending.response_started or pending.terminal)
                        )
                    if response_started:
                        wait_timeout = 0.1
                    else:
                        remaining = expires_at - time.monotonic()
                        if remaining <= 0:
                            if self._mark_timeout(request_id):
                                self._emit_timeout(request_id)
                                return self._finish_fallback(
                                    "文档预检响应超时，已使用空上下文继续翻译",
                                    preflight_request_id=request_id,
                                    warning_code="document_preflight_timeout",
                                )
                            continue
                        wait_timeout = min(0.1, remaining)
                    try:
                        response = response_queue.get(timeout=wait_timeout)
                    except queue.Empty:
                        continue
                    if response.outcome == "cancelled":
                        raise WorkerCancelled("Rust 文档预检已取消")
                    if response.outcome != "completed":
                        error_message = _preflight_error_message(response)
                        return self._finish_fallback(
                            error_message,
                            preflight_request_id=request_id,
                        )
                    if (
                        not response.context_valid
                        or not isinstance(response.document_context, dict)
                    ):
                        return self._finish_fallback(
                            "文档预检响应缺少有效的 document_context",
                            preflight_request_id=request_id,
                        )
                    return self._finish_response(response, preflight_request_id=request_id)
            finally:
                with self._pending_lock:
                    self._pending.pop(request_id, None)

    def resolve(self, message: dict[str, Any]) -> None:
        if message.get("task_id") != self.task_id:
            raise WorkerProtocolError("DOCUMENT_PREFLIGHT_RESPONSE 的 task_id 与当前任务不匹配")
        request_id = message.get("preflight_request_id")
        if not isinstance(request_id, str) or not request_id:
            raise WorkerProtocolError("DOCUMENT_PREFLIGHT_RESPONSE 缺少 preflight_request_id")
        accepted = False
        with self._pending_lock:
            pending = self._pending.get(request_id)
            if pending is None or pending.terminal:
                # A late response after the bounded fallback, cancellation, or
                # an already accepted terminal response is harmless and must
                # not turn a recoverable preflight result into a failed PDF job.
                return
            response = _document_preflight_response(message)
            try:
                pending.response_queue.put_nowait(response)
            except queue.Full:
                return
            pending.terminal = True
            accepted = True
        if accepted:
            try:
                self._emit(
                    {
                        "type": "DOCUMENT_PREFLIGHT_ACCEPTED",
                        "task_id": self.task_id,
                        "preflight_request_id": request_id,
                    }
                )
            except Exception:  # noqa: BLE001 - acceptance must not block Worker progress
                logger.exception("Unable to emit document preflight acceptance")

    def mark_activity(self, message: dict[str, Any]) -> None:
        if message.get("task_id") != self.task_id:
            raise WorkerProtocolError(
                "DOCUMENT_PREFLIGHT_ACTIVITY 的 task_id 与当前任务不匹配"
            )
        request_id = message.get("preflight_request_id")
        if not isinstance(request_id, str) or not request_id:
            raise WorkerProtocolError("DOCUMENT_PREFLIGHT_ACTIVITY 缺少 preflight_request_id")
        phase = message.get("phase")
        with self._pending_lock:
            pending = self._pending.get(request_id)
            if pending is None or pending.terminal:
                # Activity can arrive after timeout, cancellation, or a
                # terminal response. It must not reopen that request.
                return
            if not isinstance(phase, str) or phase not in {"thinking", "streaming"}:
                raise WorkerProtocolError(
                    "DOCUMENT_PREFLIGHT_ACTIVITY 的 phase 必须是 thinking 或 streaming"
                )
            pending.response_started = True

    def _terminate(self, request_id: str) -> bool:
        with self._pending_lock:
            pending = self._pending.get(request_id)
            if pending is None or pending.terminal:
                return False
            pending.terminal = True
            self._pending.pop(request_id, None)
            return True

    def _mark_timeout(self, request_id: str) -> bool:
        with self._pending_lock:
            pending = self._pending.get(request_id)
            if pending is None or pending.terminal or pending.response_started:
                return False
            pending.terminal = True
            self._pending.pop(request_id, None)
            return True

    def _emit_timeout(self, request_id: str) -> None:
        try:
            self._emit(
                {
                    "type": "DOCUMENT_PREFLIGHT_TIMEOUT",
                    "task_id": self.task_id,
                    "preflight_request_id": request_id,
                    "reason": "no_response",
                }
            )
        except Exception:  # noqa: BLE001 - timeout fallback must still proceed
            logger.exception("Unable to emit document preflight timeout")

    def _finish_response(
        self,
        response: DocumentPreflightResponse,
        *,
        preflight_request_id: str | None = None,
    ) -> DocumentContextState:
        context = _sanitize_json_value(response.document_context)
        if not isinstance(context, dict):
            return self._finish_fallback(
                "文档预检响应的 document_context 无法序列化",
                preflight_request_id=preflight_request_id,
            )
        if response.context_hash:
            context["context_hash"] = response.context_hash
        task_terms = response.task_terms or _list_field(context, "task_terms")
        if not task_terms:
            task_terms = _list_field(context, "key_terms")
        abbreviations = response.abbreviations or _list_field(context, "abbreviations")
        task_terms = _reliable_term_constraints(task_terms)
        abbreviations = _reliable_abbreviation_constraints(abbreviations)
        warnings = list(response.warnings)
        if response.degraded and not warnings:
            warnings.append("文档预检已降级，继续使用受限上下文翻译")
        state = DocumentContextState(
            document_context=context,
            context_hash=response.context_hash,
            context_before=_bounded_context_window(response.context_before),
            context_after=_bounded_context_window(response.context_after),
            task_terms=copy.deepcopy(task_terms),
            abbreviations=copy.deepcopy(abbreviations),
            warnings=warnings,
            fallback=response.degraded,
        )
        self._set_state(state)
        for warning in state.warnings:
            self._emit_warning(
                "document_preflight_warning",
                warning,
                preflight_request_id=preflight_request_id,
            )
        return self.state

    def _finish_fallback(
        self,
        message: str,
        *,
        preflight_request_id: str | None = None,
        warning_code: str = "document_preflight_failed",
    ) -> DocumentContextState:
        state = DocumentContextState(
            document_context={},
            context_hash=None,
            context_before=[],
            context_after=[],
            task_terms=[],
            abbreviations=[],
            warnings=[message],
            fallback=True,
        )
        self._set_state(state)
        self._emit_warning(
            warning_code,
            message,
            preflight_request_id=preflight_request_id,
        )
        return self.state

    def _set_state(self, state: DocumentContextState) -> None:
        with self._state_lock:
            self._state = copy.deepcopy(state)
            self._initialized = True
        if self._on_state is not None:
            self._on_state(copy.deepcopy(state))

    def _emit_warning(
        self,
        code: str,
        message: str,
        *,
        preflight_request_id: str | None = None,
    ) -> None:
        try:
            warning = {
                "type": "WARNING",
                "task_id": self.task_id,
                "code": code,
                "message": message[:500],
            }
            if preflight_request_id:
                warning["preflight_request_id"] = preflight_request_id
            self._emit(warning)
        except Exception:  # noqa: BLE001 - warning must not block translation
            logger.exception("Unable to emit document preflight warning")


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
        context_before: list[Any] | None = None,
        context_after: list[Any] | None = None,
        task_terms: list[Any] | None = None,
        abbreviations: list[Any] | None = None,
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
                    "context_before": copy.deepcopy(context_before or []),
                    "context_after": copy.deepcopy(context_after or []),
                    "task_terms": copy.deepcopy(task_terms or []),
                    "abbreviations": copy.deepcopy(abbreviations or []),
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
        preflight: DocumentPreflightCoordinator | None = None,
        engine_constraints: dict[str, Any] | None = None,
    ) -> None:
        self.lang_in = lang_in
        self.lang_out = lang_out
        self.task_id = task_id
        self.router = router
        self.cancel_event = cancel_event
        self.preflight = preflight
        self.engine_constraints = _sanitize_mapping(engine_constraints)
        self.model = "lilt-translation-core"
        self._recent_source_texts: list[str] = []
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
        state = (
            self.preflight.ensure(segments)
            if self.preflight is not None
            else _empty_context_state()
        )
        context_before = _bounded_context_window(
            [*state.context_before, *self._recent_source_texts]
        )
        context_after = state.context_after or _bounded_context_window(
            [
                segment.get("source_text")
                for segment in segments[1:]
                if isinstance(segment.get("source_text"), str)
            ]
        )
        segment_constraints = copy.deepcopy(self.engine_constraints)
        segment_constraints.update(
            {
                "response_format": "json" if batch_shape else "text",
                "placeholder_policy": "preserve",
                "request_json_mode": bool((rate_limit_params or {}).get("request_json_mode")),
            }
        )
        response = self.router.request(
            task_id=self.task_id,
            mode="pdf_segment",
            source_language=self.lang_in,
            target_language=self.lang_out,
            segments=segments,
            document_context=state.document_context,
            engine_constraints=segment_constraints,
            context_before=context_before,
            context_after=context_after,
            task_terms=state.task_terms,
            abbreviations=state.abbreviations,
        )
        self._record_usage(response.token_usage)
        self._recent_source_texts.extend(
            segment["source_text"]
            for segment in segments
            if isinstance(segment.get("source_text"), str)
        )
        self._recent_source_texts = self._recent_source_texts[-PREFLIGHT_MAX_CONTEXT_SEGMENTS:]
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
        self._document_preflight: DocumentPreflightCoordinator | None = None
        self._document_context_lock = threading.Lock()
        self._document_context: dict[str, Any] = {}
        self._context_hash: str | None = None
        self._context_before: list[Any] = []
        self._context_after: list[Any] = []
        self._task_terms: list[Any] = []
        self._abbreviations: list[Any] = []

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
        if message_type == "DOCUMENT_PREFLIGHT_RESPONSE":
            if self._document_preflight is None:
                raise WorkerProtocolError("当前任务没有等待中的文档预检")
            self._document_preflight.resolve(message)
            return
        if message_type == "DOCUMENT_PREFLIGHT_ACTIVITY":
            if self._document_preflight is None:
                raise WorkerProtocolError("当前任务没有等待中的文档预检")
            self._document_preflight.mark_activity(message)
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
        self._document_preflight = None
        with self._document_context_lock:
            self._document_context = {}
            self._context_hash = None
            self._context_before = []
            self._context_after = []
            self._task_terms = []
            self._abbreviations = []
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
            options = message.get("pdf_options")
            if not isinstance(options, dict):
                options = {}
            lang_in = str(options.get("source_language") or "en")
            lang_out = str(options.get("target_language") or "zh-CN")
            preflight = DocumentPreflightCoordinator(
                task_id=task_id,
                source_language=lang_in,
                target_language=lang_out,
                emit=self.emit,
                cancel_event=self.cancel_event,
                metadata=_preflight_metadata(options),
                engine_constraints=_preflight_engine_constraints(options),
                configured_samples=_preflight_configured_samples(options),
                timeout_seconds=_preflight_timeout(options),
                on_state=self._save_document_context,
            )
            self._document_preflight = preflight
            with redirect_stdout(sys.stderr):
                result = _run_babeldoc(
                    message,
                    input_pdf,
                    output_dir,
                    self.router,
                    self.cancel_event,
                    self.emit,
                    babeldoc_api,
                    preflight=preflight,
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

    def _save_document_context(self, state: DocumentContextState) -> None:
        with self._document_context_lock:
            self._document_context = copy.deepcopy(state.document_context)
            self._context_hash = state.context_hash
            self._context_before = copy.deepcopy(state.context_before)
            self._context_after = copy.deepcopy(state.context_after)
            self._task_terms = copy.deepcopy(state.task_terms)
            self._abbreviations = copy.deepcopy(state.abbreviations)


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
    *,
    preflight: DocumentPreflightCoordinator | None = None,
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
        preflight=preflight,
        engine_constraints=_segment_engine_constraints(options),
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
                    "placeholders": (
                        item.get("placeholders", [])
                        if isinstance(item.get("placeholders"), list)
                        else []
                    ),
                }
            )
        return segments, ids
    return [{"segment_id": "segment-0", "source_text": text, "placeholders": []}], None


def _empty_context_state() -> DocumentContextState:
    return DocumentContextState({}, None, [], [], [], [], [])


def _document_preflight_response(message: dict[str, Any]) -> DocumentPreflightResponse:
    context_value = message.get("document_context")
    context_valid = isinstance(context_value, dict)
    context = context_value
    if not context_valid:
        context = {}
    context_hash = message.get("context_hash")
    return DocumentPreflightResponse(
        outcome=str(message.get("outcome") or "failed"),
        document_context=copy.deepcopy(context),
        context_hash=context_hash if isinstance(context_hash, str) and context_hash else None,
        warnings=_string_list(message.get("warnings")),
        error=message.get("error") if isinstance(message.get("error"), dict) else None,
        context_before=_list_field(context, "context_before"),
        context_after=_list_field(context, "context_after"),
        task_terms=_list_field(context, "task_terms"),
        abbreviations=_list_field(context, "abbreviations"),
        context_valid=context_valid,
        degraded=bool(message.get("degraded", False)),
    )


def _preflight_error_message(response: DocumentPreflightResponse) -> str:
    if response.error:
        detail = response.error.get("message")
        if isinstance(detail, str) and detail.strip():
            return f"文档预检失败，已使用空上下文继续翻译：{detail.strip()[:400]}"
    if response.warnings:
        return f"文档预检失败，已使用空上下文继续翻译：{response.warnings[0][:400]}"
    return "文档预检失败，已使用空上下文继续翻译"


def _string_list(value: Any) -> list[str]:
    if not isinstance(value, list):
        return []
    return [item.strip()[:500] for item in value if isinstance(item, str) and item.strip()]


def _list_field(value: Any, key: str) -> list[Any]:
    if not isinstance(value, dict) or not isinstance(value.get(key), list):
        return []
    return copy.deepcopy(value[key])


def _reliable_term_constraints(value: Any) -> list[Any]:
    if not isinstance(value, list):
        return []
    result = []
    for item in value:
        if not isinstance(item, dict):
            continue
        source = item.get("source") or item.get("term") or item.get("original")
        target = item.get("target") or item.get("translation")
        if not isinstance(source, str) or not source.strip():
            continue
        if not isinstance(target, str) or not target.strip():
            continue
        if not _meets_confidence_threshold(item):
            continue
        result.append(copy.deepcopy(item))
    return result[:PREFLIGHT_MAX_CONSTRAINT_ITEMS]


def _reliable_abbreviation_constraints(value: Any) -> list[Any]:
    if not isinstance(value, list):
        return []
    result = []
    for item in value:
        if not isinstance(item, dict):
            continue
        abbreviation = item.get("abbreviation") or item.get("short")
        target = item.get("target") or item.get("translation")
        if not isinstance(abbreviation, str) or not abbreviation.strip():
            continue
        if not isinstance(target, str) or not target.strip():
            continue
        if not _meets_confidence_threshold(item):
            continue
        result.append(copy.deepcopy(item))
    return result[:PREFLIGHT_MAX_CONSTRAINT_ITEMS]


def _meets_confidence_threshold(item: dict[str, Any]) -> bool:
    confidence = item.get("confidence")
    if confidence is None:
        return True
    return (
        isinstance(confidence, (int, float))
        and not isinstance(confidence, bool)
        and confidence >= TASK_CONSTRAINT_MIN_CONFIDENCE
    )


def _bounded_context_window(value: Any) -> list[Any]:
    if isinstance(value, str):
        values: list[Any] = [value]
    elif isinstance(value, list):
        values = value
    else:
        return []
    result: list[Any] = []
    used_chars = 0
    for item in values:
        if len(result) >= PREFLIGHT_MAX_CONTEXT_SEGMENTS:
            break
        if isinstance(item, str):
            remaining = PREFLIGHT_MAX_CONTEXT_CHARS - used_chars
            if remaining <= 0:
                break
            item = item[:remaining]
            if not item:
                continue
            used_chars += len(item)
        else:
            try:
                item_size = len(json.dumps(item, ensure_ascii=False, separators=(",", ":")))
            except (TypeError, ValueError):
                continue
            if used_chars + item_size > PREFLIGHT_MAX_CONTEXT_CHARS:
                continue
            item = copy.deepcopy(item)
            used_chars += item_size
        result.append(item)
    return result


def _limited_preflight_samples(values: list[Any]) -> list[dict[str, Any]]:
    if not isinstance(values, list):
        return []
    samples: list[dict[str, Any]] = []
    seen: set[tuple[str, str]] = set()
    used_chars = 0
    for index, value in enumerate(values):
        if len(samples) >= PREFLIGHT_MAX_SAMPLES or used_chars >= PREFLIGHT_MAX_SAMPLE_CHARS:
            break
        if isinstance(value, str):
            sample_id = f"sample-{index}"
            source_text = value
            placeholders: list[Any] = []
            extra: dict[str, Any] = {}
        elif isinstance(value, dict):
            sample_id = str(value.get("segment_id") or value.get("id") or f"sample-{index}")
            source_text = value.get("source_text")
            if not isinstance(source_text, str):
                source_text = value.get("input")
            if not isinstance(source_text, str):
                source_text = value.get("text")
            if not isinstance(source_text, str):
                continue
            placeholders = (
                copy.deepcopy(value.get("placeholders"))
                if isinstance(value.get("placeholders"), list)
                else []
            )
            extra = {}
            for key in ("page_number", "heading_level", "layout_label", "is_heading"):
                if isinstance(value.get(key), (str, int, float, bool)):
                    extra[key] = value[key]
        else:
            continue
        if not source_text:
            continue
        identity = (sample_id, source_text)
        if identity in seen:
            continue
        seen.add(identity)
        remaining = PREFLIGHT_MAX_SAMPLE_CHARS - used_chars
        if remaining <= 0:
            break
        source_text = source_text[:remaining]
        sample = {
            "segment_id": sample_id,
            "source_text": source_text,
            "placeholders": placeholders,
        }
        sample.update(extra)
        samples.append(sample)
        used_chars += len(source_text)
    return samples


def _sanitize_json_value(value: Any, *, depth: int = 0) -> Any:
    if depth > 4:
        return None
    if isinstance(value, dict):
        result: dict[str, Any] = {}
        for key, item in list(value.items())[:32]:
            if not isinstance(key, str) or _is_sensitive_key(key):
                continue
            result[key] = _sanitize_json_value(item, depth=depth + 1)
        return result
    if isinstance(value, list):
        return [_sanitize_json_value(item, depth=depth + 1) for item in value[:32]]
    if isinstance(value, str):
        return value[:4_000]
    if value is None or isinstance(value, (bool, int, float)):
        return value
    return str(value)[:500]


def _sanitize_mapping(value: Any) -> dict[str, Any]:
    sanitized = _sanitize_json_value(value)
    return sanitized if isinstance(sanitized, dict) else {}


def _is_sensitive_key(key: str) -> bool:
    normalized = key.lower().replace("-", "_")
    return any(
        marker in normalized
        for marker in (
            "api_key",
            "apikey",
            "authorization",
            "base_url",
            "credential",
            "password",
            "provider",
            "secret",
            "token",
        )
    )


def _preflight_metadata(options: dict[str, Any]) -> dict[str, Any]:
    metadata = options.get("metadata")
    if not isinstance(metadata, dict):
        metadata = options.get("document_metadata")
    return _sanitize_mapping(metadata)


def _preflight_engine_constraints(options: dict[str, Any]) -> dict[str, Any]:
    constraints = _sanitize_mapping(options.get("engine_constraints"))
    constraints.setdefault("response_format", "json")
    constraints.setdefault("sample_limit", PREFLIGHT_MAX_SAMPLES)
    constraints.setdefault("sample_char_limit", PREFLIGHT_MAX_SAMPLE_CHARS)
    return constraints


def _segment_engine_constraints(options: dict[str, Any]) -> dict[str, Any]:
    return _sanitize_mapping(options.get("engine_constraints"))


def _preflight_configured_samples(options: dict[str, Any]) -> list[Any]:
    value = options.get("samples")
    return value if isinstance(value, list) else []


def _preflight_timeout(options: dict[str, Any]) -> float:
    value = options.get("preflight_timeout_seconds")
    if isinstance(value, (int, float)) and not isinstance(value, bool):
        return float(value)
    return PREFLIGHT_TIMEOUT_SECONDS


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
