import json
import unittest
from urllib.error import HTTPError
from urllib.request import Request, urlopen

from provider_stub import ProviderStub, StubConfig


class ProviderStubTests(unittest.TestCase):
    def setUp(self):
        self.stub = ProviderStub().start()

    def tearDown(self):
        self.stub.close()

    def test_models_endpoint_is_deterministic(self):
        request = Request(
            f"{self.stub.base_url}/models",
            headers={"Authorization": "Bearer test-key"},
        )
        with urlopen(request, timeout=2) as response:
            payload = json.load(response)
        self.assertEqual(payload["data"][0]["id"], "stub-model")

    def test_chat_endpoint_returns_sse(self):
        payload = {
            "model": "stub-model",
            "stream": True,
            "reasoning_effort": "none",
            "messages": [{"role": "user", "content": "hello"}],
        }
        request = Request(
            f"{self.stub.base_url}/chat/completions",
            data=json.dumps(payload).encode("utf-8"),
            headers={
                "Authorization": "Bearer test-key",
                "Content-Type": "application/json",
            },
            method="POST",
        )
        with urlopen(request, timeout=2) as response:
            lines = response.read().decode("utf-8").splitlines()
        deltas = [
            json.loads(line.removeprefix("data: "))["choices"][0]["delta"].get("content", "")
            for line in lines
            if line.startswith("data: ")
            and line != "data: [DONE]"
            and json.loads(line.removeprefix("data: ")).get("choices")
        ]
        self.assertEqual("".join(deltas), "【Stub】hello")

    def test_batch_content_preserves_ids(self):
        source = json.dumps(
            [{"id": "p1-s1", "input": "one"}, {"id": "p1-s2", "input": "two"}],
            ensure_ascii=False,
        )
        payload = {
            "model": "stub-model",
            "messages": [{"role": "user", "content": source}],
        }
        request = Request(
            f"{self.stub.base_url}/chat/completions",
            data=json.dumps(payload).encode("utf-8"),
            headers={
                "Authorization": "Bearer test-key",
                "Content-Type": "application/json",
            },
            method="POST",
        )
        with urlopen(request, timeout=2) as response:
            lines = response.read().decode("utf-8").splitlines()
        text = "".join(
            json.loads(line.removeprefix("data: "))["choices"][0]["delta"].get("content", "")
            for line in lines
            if line.startswith("data: ")
            and line != "data: [DONE]"
            and json.loads(line.removeprefix("data: ")).get("choices")
        )
        result = json.loads(text.removeprefix("【Stub】"))
        self.assertEqual([item["id"] for item in result], ["p1-s1", "p1-s2"])

    def test_rejects_wrong_api_key(self):
        request = Request(
            f"{self.stub.base_url}/models",
            headers={"Authorization": "Bearer wrong-key"},
        )
        with self.assertRaises(HTTPError) as context:
            urlopen(request, timeout=2)
        self.assertEqual(context.exception.code, 401)

    def test_can_emit_an_http_failure(self):
        stub = ProviderStub(StubConfig(failure_mode="http")).start()
        self.addCleanup(stub.close)
        payload = {
            "model": "stub-model",
            "messages": [{"role": "user", "content": "hello"}],
        }
        request = Request(
            f"{stub.base_url}/chat/completions",
            data=json.dumps(payload).encode("utf-8"),
            headers={
                "Authorization": "Bearer test-key",
                "Content-Type": "application/json",
            },
            method="POST",
        )
        with self.assertRaises(HTTPError) as context:
            urlopen(request, timeout=2)
        self.assertEqual(context.exception.code, 500)


if __name__ == "__main__":
    unittest.main()
