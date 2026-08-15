import threading
import unittest

from worker import (
    MAX_LINE_BYTES,
    LiltTranslator,
    ResponseRouter,
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


if __name__ == "__main__":
    unittest.main()
