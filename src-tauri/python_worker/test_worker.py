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
    MAX_LINE_BYTES,
    LiltTranslator,
    ResponseRouter,
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


def _start_job_message(input_pdf: pathlib.Path, output_dir: pathlib.Path) -> dict:
    return {
        "type": "START_JOB",
        "protocol_version": 1,
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
                if event.get("type") == "TRANSLATE_REQUEST":
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
