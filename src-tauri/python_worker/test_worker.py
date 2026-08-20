import json
import os
import pathlib
import queue
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from unittest import mock

from worker import (
    BabelDocWorker,
    DocumentPreflightCoordinator,
    MAX_LINE_BYTES,
    LiltTranslator,
    PREFLIGHT_TIMEOUT_SECONDS,
    ResponseRouter,
    WorkerCancelled,
    WorkerEngineUnavailable,
    WorkerProtocolError,
    decode_message,
    encode_message,
)


class WorkerProtocolTests(unittest.TestCase):
    def test_jsonl_round_trip_is_utf8_and_single_line(self):
        encoded = encode_message({"type": "PROGRESS", "message": "中文"})
        self.assertEqual(encoded.count(b"\n"), 1)
        self.assertEqual(decode_message(encoded), {"type": "PROGRESS", "message": "中文"})

    def test_rejects_multiple_frames(self):
        with self.assertRaises(WorkerProtocolError):
            decode_message(b'{"type":"A"}\n{"type":"B"}\n')

    def test_rejects_oversized_message(self):
        with self.assertRaises(WorkerProtocolError):
            encode_message({"type": "WARNING", "message": "x" * MAX_LINE_BYTES})


class ResponseRouterTests(unittest.TestCase):
    def test_response_is_matched_by_request_id(self):
        emitted = []
        cancel = threading.Event()
        router = ResponseRouter(emitted.append, cancel)
        result = {}

        def request():
            result["value"] = router.request(
                task_id="task-1",
                mode="pdf_segment",
                source_language="en",
                target_language="zh-CN",
                segments=[{"segment_id": "p1-s1", "source_text": "hello"}],
            )

        thread = threading.Thread(target=request)
        thread.start()
        while not emitted:
            pass
        request_id = emitted[0]["translation_request_id"]
        router.resolve(
            {
                "type": "TRANSLATE_RESPONSE",
                "task_id": "task-1",
                "translation_request_id": request_id,
                "outcome": "completed",
                "translated_text": "你好",
            }
        )
        thread.join(timeout=1)
        self.assertEqual(result["value"].translated_text, "你好")

    def test_translator_extracts_engine_prompt_before_ipc(self):
        emitted = []
        cancel = threading.Event()
        router = ResponseRouter(emitted.append, cancel)
        translator = LiltTranslator(
            task_id="task-1",
            lang_in="en",
            lang_out="zh-CN",
            router=router,
            cancel_event=cancel,
        )
        result = {}

        def request():
            result["value"] = translator.llm_translate(
                "rules\n\n## Here is the input:\n"
                '[{"id": 7, "input": "hello", "layout_label": "text"}]',
                rate_limit_params={"request_json_mode": True},
            )

        thread = threading.Thread(target=request)
        thread.start()
        while not emitted:
            pass
        request_id = emitted[0]["translation_request_id"]
        self.assertEqual(emitted[0]["segments"][0]["segment_id"], "7")
        self.assertEqual(emitted[0]["segments"][0]["source_text"], "hello")
        router.resolve(
            {
                "type": "TRANSLATE_RESPONSE",
                "task_id": "task-1",
                "translation_request_id": request_id,
                "outcome": "completed",
                "translated_segments":[{"segment_id":"7","translated_text":"你好"}],
            }
        )
        thread.join(timeout=1)
        self.assertEqual(result["value"], '[{"id": "7", "output": "你好"}]')


class DocumentPreflightTests(unittest.TestCase):
    def _wait_for_type(self, emitted, message_type):
        deadline = time.monotonic() + 2
        while time.monotonic() < deadline:
            for message in emitted:
                if message.get("type") == message_type:
                    return message
            time.sleep(0.001)
        self.fail(f"did not observe {message_type}: {emitted!r}")

    def test_preflight_request_and_response_save_task_context(self):
        emitted = []
        cancel = threading.Event()
        worker = BabelDocWorker(emit=emitted.append)
        coordinator = DocumentPreflightCoordinator(
            task_id="task-1",
            source_language="en",
            target_language="zh-CN",
            emit=emitted.append,
            cancel_event=cancel,
            metadata={"title": "A paper"},
            configured_samples=[{"id": "configured-1", "input": "abstract"}],
            timeout_seconds=1,
            on_state=worker._save_document_context,
        )
        result = {}
        thread = threading.Thread(
            target=lambda: result.setdefault(
                "state",
                coordinator.ensure(
                    [{"segment_id": "p1-s1", "source_text": "representative paragraph"}]
                ),
            )
        )
        thread.start()
        request = self._wait_for_type(emitted, "DOCUMENT_PREFLIGHT_REQUEST")
        self.assertEqual(request["task_id"], "task-1")
        self.assertEqual(request["source_language"], "en")
        self.assertEqual(request["target_language"], "zh-CN")
        self.assertEqual(request["metadata"], {"title": "A paper"})
        self.assertEqual(request["samples"][0]["source_text"], "abstract")
        self.assertEqual(request["samples"][1]["source_text"], "representative paragraph")
        self.assertEqual(request["engine_constraints"]["response_format"], "json")
        coordinator.resolve(
            {
                "type": "DOCUMENT_PREFLIGHT_RESPONSE",
                "task_id": "task-1",
                "preflight_request_id": request["preflight_request_id"],
                "outcome": "completed",
                "document_context": {
                    "schema_version": 1,
                    "title": "A paper",
                    "key_terms": [{"source": "term", "target": "术语"}],
                    "abbreviations": [{
                        "short": "API",
                        "expanded": "Application",
                        "target": "应用",
                    }],
                },
                "context_hash": "ctx-1",
                "warnings": [],
                "error": None,
            }
        )
        thread.join(timeout=1)
        self.assertFalse(thread.is_alive())
        state = result["state"]
        self.assertEqual(state.document_context["context_hash"], "ctx-1")
        self.assertEqual(state.context_hash, "ctx-1")
        self.assertEqual(state.task_terms[0]["target"], "术语")
        self.assertEqual(state.abbreviations[0]["short"], "API")
        self.assertFalse(state.fallback)
        self.assertEqual(worker._document_context["title"], "A paper")
        self.assertEqual(worker._context_hash, "ctx-1")

    def test_low_confidence_context_items_are_kept_out_of_constraints(self):
        emitted = []
        coordinator = DocumentPreflightCoordinator(
            task_id="task-1",
            source_language="en",
            target_language="zh-CN",
            emit=emitted.append,
            cancel_event=threading.Event(),
            timeout_seconds=1,
        )
        result = {}
        thread = threading.Thread(
            target=lambda: result.setdefault(
                "state",
                coordinator.ensure([{"segment_id": "p1-s1", "source_text": "hello"}]),
            )
        )
        thread.start()
        request = self._wait_for_type(emitted, "DOCUMENT_PREFLIGHT_REQUEST")
        coordinator.resolve(
            {
                "type": "DOCUMENT_PREFLIGHT_RESPONSE",
                "task_id": "task-1",
                "preflight_request_id": request["preflight_request_id"],
                "outcome": "completed",
                "document_context": {
                    "key_terms": [
                        {"source": "reliable", "target": "可靠", "confidence": 0.9},
                        {"source": "uncertain", "target": "不确定", "confidence": 0.4},
                    ],
                    "abbreviations": [
                        {"abbreviation": "API", "target": "接口", "confidence": 0.9},
                        {"abbreviation": "CPU", "target": "处理器", "confidence": 0.4},
                        {"abbreviation": "GPU", "expanded": "Graphics Processing Unit"},
                    ],
                },
                "context_hash": "ctx-2",
                "warnings": [],
                "error": None,
            }
        )
        thread.join(timeout=1)
        self.assertFalse(thread.is_alive())
        state = result["state"]
        self.assertEqual([item["source"] for item in state.task_terms], ["reliable"])
        self.assertEqual([item["abbreviation"] for item in state.abbreviations], ["API"])

    def test_pdf_segment_receives_context_fields_after_preflight(self):
        emitted = []
        cancel = threading.Event()
        coordinator = DocumentPreflightCoordinator(
            task_id="task-1",
            source_language="en",
            target_language="zh-CN",
            emit=emitted.append,
            cancel_event=cancel,
            timeout_seconds=1,
        )
        router = ResponseRouter(emitted.append, cancel)
        translator = LiltTranslator(
            task_id="task-1",
            lang_in="en",
            lang_out="zh-CN",
            router=router,
            cancel_event=cancel,
            preflight=coordinator,
        )
        result = {}
        thread = threading.Thread(
            target=lambda: result.setdefault("value", translator.translate("hello"))
        )
        thread.start()
        preflight = self._wait_for_type(emitted, "DOCUMENT_PREFLIGHT_REQUEST")
        coordinator.resolve(
            {
                "type": "DOCUMENT_PREFLIGHT_RESPONSE",
                "task_id": "task-1",
                "preflight_request_id": preflight["preflight_request_id"],
                "outcome": "completed",
                "document_context": {"title": "A paper"},
                "context_hash": "ctx-1",
                "warnings": [],
                "error": None,
            }
        )
        request = self._wait_for_type(emitted, "TRANSLATE_REQUEST")
        self.assertEqual(request["mode"], "pdf_segment")
        self.assertEqual(request["document_context"]["title"], "A paper")
        self.assertEqual(request["context_before"], [])
        self.assertEqual(request["context_after"], [])
        self.assertEqual(request["task_terms"], [])
        self.assertEqual(request["abbreviations"], [])
        router.resolve(
            {
                "type": "TRANSLATE_RESPONSE",
                "task_id": "task-1",
                "translation_request_id": request["translation_request_id"],
                "outcome": "completed",
                "translated_text": "你好",
            }
        )
        thread.join(timeout=1)
        self.assertFalse(thread.is_alive())
        self.assertEqual(result["value"], "你好")

    def test_preflight_failure_emits_warning_and_falls_back_to_empty_context(self):
        emitted = []
        cancel = threading.Event()
        coordinator = DocumentPreflightCoordinator(
            task_id="task-1",
            source_language="en",
            target_language="zh-CN",
            emit=emitted.append,
            cancel_event=cancel,
            timeout_seconds=1,
        )
        router = ResponseRouter(emitted.append, cancel)
        translator = LiltTranslator(
            task_id="task-1",
            lang_in="en",
            lang_out="zh-CN",
            router=router,
            cancel_event=cancel,
            preflight=coordinator,
        )
        result = {}
        thread = threading.Thread(
            target=lambda: result.setdefault("value", translator.translate("hello"))
        )
        thread.start()
        preflight = self._wait_for_type(emitted, "DOCUMENT_PREFLIGHT_REQUEST")
        coordinator.resolve(
            {
                "type": "DOCUMENT_PREFLIGHT_RESPONSE",
                "task_id": "task-1",
                "preflight_request_id": preflight["preflight_request_id"],
                "outcome": "failed",
                "document_context": {},
                "context_hash": None,
                "warnings": [],
                "error": {"code": "provider_failed", "message": "preflight unavailable"},
            }
        )
        request = self._wait_for_type(emitted, "TRANSLATE_REQUEST")
        warning = self._wait_for_type(emitted, "WARNING")
        self.assertEqual(warning["code"], "document_preflight_failed")
        self.assertEqual(request["document_context"], {})
        self.assertEqual(request["task_terms"], [])
        router.resolve(
            {
                "type": "TRANSLATE_RESPONSE",
                "task_id": "task-1",
                "translation_request_id": request["translation_request_id"],
                "outcome": "completed",
                "translated_text": "你好",
            }
        )
        thread.join(timeout=1)
        self.assertFalse(thread.is_alive())
        self.assertEqual(result["value"], "你好")
        self.assertEqual(warning["preflight_request_id"], preflight["preflight_request_id"])

    def test_default_preflight_timeout_is_sixty_seconds(self):
        self.assertEqual(PREFLIGHT_TIMEOUT_SECONDS, 60.0)

    def test_no_response_timeout_notifies_rust_and_degrades(self):
        emitted = []
        coordinator = DocumentPreflightCoordinator(
            task_id="task-1",
            source_language="en",
            target_language="zh-CN",
            emit=emitted.append,
            cancel_event=threading.Event(),
            timeout_seconds=0.1,
        )
        result = {}
        thread = threading.Thread(
            target=lambda: result.setdefault(
                "state",
                coordinator.ensure([{"segment_id": "p1-s1", "source_text": "hello"}]),
            )
        )
        thread.start()
        request = self._wait_for_type(emitted, "DOCUMENT_PREFLIGHT_REQUEST")
        thread.join(timeout=1)
        self.assertFalse(thread.is_alive())
        self.assertTrue(result["state"].fallback)

        timeout = self._wait_for_type(emitted, "DOCUMENT_PREFLIGHT_TIMEOUT")
        self.assertEqual(timeout["task_id"], "task-1")
        self.assertEqual(timeout["preflight_request_id"], request["preflight_request_id"])
        self.assertEqual(timeout["reason"], "no_response")
        warning = self._wait_for_type(emitted, "WARNING")
        self.assertEqual(warning["code"], "document_preflight_timeout")
        self.assertEqual(warning["preflight_request_id"], request["preflight_request_id"])

    def test_activity_disables_no_response_timeout_until_terminal_response(self):
        emitted = []
        coordinator = DocumentPreflightCoordinator(
            task_id="task-1",
            source_language="en",
            target_language="zh-CN",
            emit=emitted.append,
            cancel_event=threading.Event(),
            timeout_seconds=0.1,
        )
        result = {}
        thread = threading.Thread(
            target=lambda: result.setdefault(
                "state",
                coordinator.ensure([{"segment_id": "p1-s1", "source_text": "hello"}]),
            )
        )
        thread.start()
        request = self._wait_for_type(emitted, "DOCUMENT_PREFLIGHT_REQUEST")
        activity = {
            "type": "DOCUMENT_PREFLIGHT_ACTIVITY",
            "task_id": "task-1",
            "preflight_request_id": request["preflight_request_id"],
            "phase": "thinking",
        }
        coordinator.mark_activity(activity)
        time.sleep(0.2)
        self.assertTrue(thread.is_alive())
        self.assertFalse(any(event["type"] == "DOCUMENT_PREFLIGHT_TIMEOUT" for event in emitted))

        coordinator.mark_activity({**activity, "phase": "streaming"})
        coordinator.resolve(
            {
                "type": "DOCUMENT_PREFLIGHT_RESPONSE",
                "task_id": "task-1",
                "preflight_request_id": request["preflight_request_id"],
                "outcome": "completed",
                "document_context": {"title": "A paper"},
                "context_hash": "ctx-activity",
                "warnings": [],
                "error": None,
            }
        )
        thread.join(timeout=1)
        self.assertFalse(thread.is_alive())
        self.assertEqual(result["state"].document_context["title"], "A paper")
        self.assertFalse(any(event["type"] == "DOCUMENT_PREFLIGHT_TIMEOUT" for event in emitted))

    def test_late_activity_and_response_are_ignored_after_timeout(self):
        emitted = []
        coordinator = DocumentPreflightCoordinator(
            task_id="task-1",
            source_language="en",
            target_language="zh-CN",
            emit=emitted.append,
            cancel_event=threading.Event(),
            timeout_seconds=0.1,
        )
        result = {}
        thread = threading.Thread(
            target=lambda: result.setdefault(
                "state",
                coordinator.ensure([{"segment_id": "p1-s1", "source_text": "hello"}]),
            )
        )
        thread.start()
        request = self._wait_for_type(emitted, "DOCUMENT_PREFLIGHT_REQUEST")
        thread.join(timeout=1)
        self.assertFalse(thread.is_alive())
        event_count = len(emitted)

        late_response = {
            "type": "DOCUMENT_PREFLIGHT_RESPONSE",
            "task_id": "task-1",
            "preflight_request_id": request["preflight_request_id"],
            "outcome": "completed",
            "document_context": {"title": "late"},
            "context_hash": "late-context",
            "warnings": [],
            "error": None,
        }
        coordinator.mark_activity(
            {
                "type": "DOCUMENT_PREFLIGHT_ACTIVITY",
                "task_id": "task-1",
                "preflight_request_id": request["preflight_request_id"],
                "phase": "streaming",
            }
        )
        coordinator.resolve(late_response)
        coordinator.resolve(late_response)
        self.assertEqual(len(emitted), event_count)
        self.assertEqual(result["state"].document_context, {})
        self.assertTrue(result["state"].fallback)

    def test_worker_routes_preflight_activity_messages(self):
        emitted = []
        coordinator = DocumentPreflightCoordinator(
            task_id="task-1",
            source_language="en",
            target_language="zh-CN",
            emit=emitted.append,
            cancel_event=threading.Event(),
            timeout_seconds=1,
        )
        worker = BabelDocWorker(emit=emitted.append)
        worker._document_preflight = coordinator
        result = {}
        thread = threading.Thread(
            target=lambda: result.setdefault(
                "state",
                coordinator.ensure([{"segment_id": "p1-s1", "source_text": "hello"}]),
            )
        )
        thread.start()
        request = self._wait_for_type(emitted, "DOCUMENT_PREFLIGHT_REQUEST")
        worker.handle(
            {
                "type": "DOCUMENT_PREFLIGHT_ACTIVITY",
                "task_id": "task-1",
                "preflight_request_id": request["preflight_request_id"],
                "phase": "thinking",
            }
        )
        worker.handle(
            {
                "type": "DOCUMENT_PREFLIGHT_RESPONSE",
                "task_id": "task-1",
                "preflight_request_id": request["preflight_request_id"],
                "outcome": "completed",
                "document_context": {},
                "warnings": [],
                "error": None,
            }
        )
        accepted = self._wait_for_type(emitted, "DOCUMENT_PREFLIGHT_ACCEPTED")
        self.assertEqual(accepted["task_id"], "task-1")
        self.assertEqual(accepted["preflight_request_id"], request["preflight_request_id"])
        thread.join(timeout=1)
        self.assertFalse(thread.is_alive())
        self.assertFalse(result["state"].fallback)

    def test_preflight_wait_is_cancelled(self):
        emitted = []
        cancel = threading.Event()
        coordinator = DocumentPreflightCoordinator(
            task_id="task-1",
            source_language="en",
            target_language="zh-CN",
            emit=emitted.append,
            cancel_event=cancel,
            timeout_seconds=1,
        )
        result = {}
        thread = threading.Thread(
            target=lambda: self._capture_exception(
                result, lambda: coordinator.ensure([{"source_text": "hello"}])
            )
        )
        thread.start()
        self._wait_for_type(emitted, "DOCUMENT_PREFLIGHT_REQUEST")
        cancel.set()
        thread.join(timeout=1)
        self.assertFalse(thread.is_alive())
        self.assertIsInstance(result["error"], WorkerCancelled)

    @staticmethod
    def _capture_exception(result, operation):
        try:
            result["value"] = operation()
        except Exception as exc:  # noqa: BLE001 - test helper
            result["error"] = exc


def _start_job_message(input_pdf: pathlib.Path, output_dir: pathlib.Path) -> dict:
    return {
        "type": "START_JOB",
        "protocol_version": 2,
        "task_id": "task-1",
        "input_pdf": str(input_pdf),
        "output_dir": str(output_dir),
        "engine_version": "babeldoc-0.6.4",
        "pdf_options": {"source_language": "en", "target_language": "zh-CN"},
    }


class WorkerStartupTests(unittest.TestCase):
    def test_babeldoc_api_is_loaded_on_main_thread_before_job_thread(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            root = pathlib.Path(temp_dir)
            input_pdf = root / "input.pdf"
            input_pdf.write_bytes(b"%PDF-1.7\n")
            output_dir = root / "output"
            load_events = []
            fake_api = object()

            def load_api():
                load_events.append(("load", threading.current_thread().name))
                return fake_api

            def run_job(_message, _input_pdf, _output_dir, babeldoc_api):
                load_events.append(
                    ("run", threading.current_thread().name, babeldoc_api)
                )

            worker = BabelDocWorker(emit=lambda _message: None)
            with (
                mock.patch("worker._load_babeldoc_api", side_effect=load_api),
                mock.patch.object(worker, "_run_job", side_effect=run_job),
            ):
                worker._start_job(_start_job_message(input_pdf, output_dir))

            self.assertIsNotNone(worker._job_thread)
            worker._job_thread.join(timeout=1)
            self.assertEqual(load_events[0][0], "load")
            self.assertEqual(load_events[0][1], threading.current_thread().name)
            self.assertEqual(load_events[1][0], "run")
            self.assertIs(load_events[1][2], fake_api)

    def test_missing_engine_emits_structured_error_without_starting_job_thread(self):
        events = []
        with tempfile.TemporaryDirectory() as temp_dir:
            root = pathlib.Path(temp_dir)
            input_pdf = root / "input.pdf"
            input_pdf.write_bytes(b"%PDF-1.7\n")
            worker = BabelDocWorker(emit=events.append)
            with mock.patch(
                "worker._load_babeldoc_api",
                side_effect=WorkerEngineUnavailable("cv2 import failed"),
            ):
                worker._start_job(_start_job_message(input_pdf, root / "output"))

        self.assertIsNone(worker._job_thread)
        error = next(event for event in events if event["type"] == "ERROR")
        self.assertEqual(error["error"]["code"], "engine_unavailable")
        self.assertIn("cv2 import failed", error["error"]["message"])


@unittest.skipUnless(
    os.environ.get("LILT_RUN_PDF_WORKER_INTEGRATION") == "1",
    "set LILT_RUN_PDF_WORKER_INTEGRATION=1 to run the local BabelDOC harness",
)
class WorkerSubprocessIntegrationTests(unittest.TestCase):
    def test_real_worker_subprocess_with_local_translation_responder(self):
        project_root = pathlib.Path(__file__).resolve().parents[2]
        input_pdf = (
            project_root
            / "reference-projects"
            / "PDFMathTranslate-next"
            / "test"
            / "file"
            / "translate.cli.plain.text.pdf"
        )
        if not input_pdf.is_file():
            self.skipTest(f"missing local fixture: {input_pdf}")

        events: queue.Queue[tuple[str, str]] = queue.Queue()
        worker_script = project_root / "src-tauri" / "python_worker" / "worker.py"
        worker_command = [sys.executable, "-u", str(worker_script)]
        worker_process = subprocess.Popen(
            worker_command,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            encoding="utf-8",
            errors="replace",
            bufsize=1,
        )

        def read_stream(stream, kind):
            for line in stream:
                events.put((kind, line))

        threading.Thread(
            target=read_stream,
            args=(worker_process.stdout, "stdout"),
            daemon=True,
        ).start()
        threading.Thread(
            target=read_stream,
            args=(worker_process.stderr, "stderr"),
            daemon=True,
        ).start()

        with tempfile.TemporaryDirectory() as temp_dir:
            output_dir = pathlib.Path(temp_dir) / "output"
            worker_process.stdin.write(
                json.dumps(
                    _start_job_message(input_pdf, output_dir),
                    ensure_ascii=False,
                )
                + "\n"
            )
            worker_process.stdin.flush()

            observed_types = []
            stderr_lines = []
            terminal_event = None
            timeout_seconds = float(os.environ.get("LILT_PDF_INTEGRATION_TIMEOUT", "180"))
            deadline = time.monotonic() + timeout_seconds
            while time.monotonic() < deadline and terminal_event is None:
                try:
                    stream_kind, line = events.get(timeout=1)
                except queue.Empty:
                    if worker_process.poll() is not None:
                        self.fail(
                            f"Worker exited before terminal event: {worker_process.returncode}"
                        )
                    continue
                if stream_kind != "stdout":
                    stderr_lines.append(line.rstrip())
                    continue

                try:
                    event = json.loads(line)
                except json.JSONDecodeError as exc:
                    worker_process.terminate()
                    worker_process.wait(timeout=5)
                    self.fail(f"Worker emitted non-JSON stdout: {line!r}: {exc}")
                observed_types.append(event.get("type"))
                if event.get("type") == "DOCUMENT_PREFLIGHT_REQUEST":
                    response = {
                        "type": "DOCUMENT_PREFLIGHT_RESPONSE",
                        "task_id": event.get("task_id"),
                        "preflight_request_id": event.get("preflight_request_id"),
                        "outcome": "completed",
                        "document_context": {
                            "schema_version": 1,
                            "title": "Local fixture",
                        },
                        "context_hash": "integration-context",
                        "warnings": [],
                        "error": None,
                    }
                    worker_process.stdin.write(
                        json.dumps(response, ensure_ascii=False) + "\n"
                    )
                    worker_process.stdin.flush()
                elif event.get("type") == "TRANSLATE_REQUEST":
                    segments = event.get("segments") or []
                    translated_segments = [
                        {
                            "segment_id": segment.get("segment_id"),
                            "translated_text": "【译】" + segment.get("source_text", ""),
                        }
                        for segment in segments
                    ]
                    response = {
                        "type": "TRANSLATE_RESPONSE",
                        "task_id": event.get("task_id"),
                        "translation_request_id": event.get("translation_request_id"),
                        "outcome": "completed",
                        "translated_text": (
                            translated_segments[0]["translated_text"]
                            if len(segments) == 1
                            else None
                        ),
                        "translated_segments": translated_segments,
                        "token_usage": {},
                        "cache_hit": False,
                        "warnings": [],
                    }
                    worker_process.stdin.write(
                        json.dumps(response, ensure_ascii=False) + "\n"
                    )
                    worker_process.stdin.flush()
                elif event.get("type") in {"FINISHED", "CANCELLED", "ERROR"}:
                    terminal_event = event

            if terminal_event is None:
                self.fail(
                    "Worker did not emit a terminal event. Recent stderr:\n"
                    + "\n".join(stderr_lines[-80:])
                )
            self.assertIn("DOCUMENT_PREFLIGHT_REQUEST", observed_types)
            self.assertIn("TRANSLATE_REQUEST", observed_types)
            self.assertEqual(terminal_event["type"], "FINISHED")
            output_pdf = pathlib.Path(terminal_event["output_pdf"])
            self.assertTrue(output_pdf.is_file())
            self.assertTrue(output_pdf.read_bytes().startswith(b"%PDF-"))

        worker_process.stdin.close()
        try:
            worker_process.wait(timeout=10)
        except subprocess.TimeoutExpired:
            worker_process.terminate()
            worker_process.wait(timeout=5)
        finally:
            worker_process.stdout.close()
            worker_process.stderr.close()


if __name__ == "__main__":
    unittest.main()
