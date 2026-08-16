"""Deterministic local OpenAI-compatible Provider Stub for PDF E2E tests."""

from __future__ import annotations

import argparse
import json
import threading
import time
from dataclasses import dataclass
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import Any
from urllib.parse import urlparse


@dataclass(frozen=True)
class StubConfig:
    api_key: str = "test-key"
    model_id: str = "stub-model"
    delay_ms: int = 0
    failure_mode: str | None = None
    prefix: str = "【Stub】"


class ProviderStub:
    """A small in-process HTTP server with a controllable test surface."""

    def __init__(self, config: StubConfig | None = None):
        self.config = config or StubConfig()
        config = self.config

        class Handler(_ProviderRequestHandler):
            stub_config = config

        self.server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.thread: threading.Thread | None = None

    @property
    def port(self) -> int:
        return int(self.server.server_address[1])

    @property
    def base_url(self) -> str:
        return f"http://127.0.0.1:{self.port}/v1"

    def start(self) -> "ProviderStub":
        if self.thread is not None:
            return self
        self.thread = threading.Thread(
            target=self.server.serve_forever,
            name="lilt-provider-stub",
            daemon=True,
        )
        self.thread.start()
        return self

    def close(self) -> None:
        self.server.shutdown()
        self.server.server_close()
        if self.thread is not None:
            self.thread.join(timeout=2)
            self.thread = None

    def __enter__(self) -> "ProviderStub":
        return self.start()

    def __exit__(self, _exc_type: Any, _exc: Any, _traceback: Any) -> None:
        self.close()


class _ProviderRequestHandler(BaseHTTPRequestHandler):
    stub_config: StubConfig

    def log_message(self, _format: str, *_args: Any) -> None:
        return

    def do_GET(self) -> None:  # noqa: N802 - BaseHTTPRequestHandler API
        if urlparse(self.path).path != "/v1/models":
            self._write_json(404, {"error": {"message": "not found"}})
            return
        if not self._authorized():
            self._write_json(401, {"error": {"message": "invalid api key"}})
            return
        self._write_json(
            200,
            {
                "object": "list",
                "data": [
                    {
                        "id": self.stub_config.model_id,
                        "object": "model",
                        "owned_by": "lilt-test",
                    }
                ],
            },
        )

    def do_POST(self) -> None:  # noqa: N802 - BaseHTTPRequestHandler API
        if urlparse(self.path).path != "/v1/chat/completions":
            self._write_json(404, {"error": {"message": "not found"}})
            return
        if not self._authorized():
            self._write_json(401, {"error": {"message": "invalid api key"}})
            return

        mode = self.stub_config.failure_mode
        if mode == "http":
            self._write_json(500, {"error": {"message": "stub failure"}})
            return

        length = int(self.headers.get("Content-Length", "0"))
        try:
            payload = json.loads(self.rfile.read(length).decode("utf-8"))
        except (UnicodeDecodeError, json.JSONDecodeError) as error:
            self._write_json(400, {"error": {"message": f"invalid request: {error}"}})
            return

        if payload.get("model") != self.stub_config.model_id:
            self._write_json(400, {"error": {"message": "unexpected model"}})
            return
        if self.stub_config.delay_ms > 0:
            time.sleep(self.stub_config.delay_ms / 1000)

        source = _user_content(payload)
        if mode == "invalid_sse":
            self._write_sse_body(b"data: {invalid-json}\n\n")
            return
        if mode == "disconnect":
            body = _sse_body(self.stub_config.prefix + source)
            self.send_response(200)
            self.send_header("Content-Type", "text/event-stream")
            self.send_header("Connection", "close")
            self.end_headers()
            self.wfile.write(body[: max(1, len(body) // 2)])
            self.wfile.flush()
            self.close_connection = True
            return

        try:
            translated = _translate(source, self.stub_config.prefix)
        except (TypeError, ValueError) as error:
            self._write_json(400, {"error": {"message": str(error)}})
            return
        self._write_sse_body(_sse_body(translated))

    def _authorized(self) -> bool:
        expected = self.stub_config.api_key
        return not expected or self.headers.get("Authorization") == f"Bearer {expected}"

    def _write_json(self, status: int, payload: dict[str, Any]) -> None:
        body = json.dumps(payload, ensure_ascii=False).encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type", "application/json; charset=utf-8")
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Connection", "close")
        self.end_headers()
        self.wfile.write(body)

    def _write_sse_body(self, body: bytes) -> None:
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream; charset=utf-8")
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Connection", "close")
        self.end_headers()
        self.wfile.write(body)


def _user_content(payload: dict[str, Any]) -> str:
    messages = payload.get("messages")
    if not isinstance(messages, list) or not messages:
        raise ValueError("messages must be a non-empty list")
    content = messages[-1].get("content")
    if not isinstance(content, str):
        raise ValueError("last message content must be a string")
    return content


def _translate(source: str, prefix: str) -> str:
    try:
        items = json.loads(source)
    except json.JSONDecodeError:
        return prefix + source
    if not isinstance(items, list):
        return prefix + source
    translated = []
    for item in items:
        if not isinstance(item, dict) or "id" not in item or not isinstance(item.get("input"), str):
            raise ValueError("batch item must contain id and input")
        translated.append({"id": item["id"], "output": prefix + item["input"]})
    return json.dumps(translated, ensure_ascii=False)


def _sse_body(content: str) -> bytes:
    chunks = [content[index : index + 16] for index in range(0, len(content), 16)] or [""]
    lines = [
        "data: "
        + json.dumps({"choices": [{"delta": {"content": chunk}}]}, ensure_ascii=False)
        + "\n\n"
        for chunk in chunks
    ]
    lines.append("data: [DONE]\n\n")
    lines.insert(
        -1,
        "data: "
        + json.dumps(
            {
                "choices": [],
                "usage": {
                    "prompt_tokens": 10,
                    "completion_tokens": 4,
                    "total_tokens": 14,
                },
            }
        )
        + "\n\n",
    )
    return "".join(lines).encode("utf-8")


def _parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Lilt local OpenAI-compatible Provider Stub")
    parser.add_argument("--port", type=int, default=0)
    parser.add_argument("--api-key", default="test-key")
    parser.add_argument("--model-id", default="stub-model")
    parser.add_argument("--delay-ms", type=int, default=0)
    parser.add_argument(
        "--failure-mode",
        choices=["http", "invalid_sse", "disconnect"],
        default=None,
    )
    return parser.parse_args()


def main() -> None:
    args = _parse_args()
    stub = ProviderStub(
        StubConfig(
            api_key=args.api_key,
            model_id=args.model_id,
            delay_ms=max(0, args.delay_ms),
            failure_mode=args.failure_mode,
        )
    )
    if args.port:
        stub.server.server_close()
        stub.server = ThreadingHTTPServer(("127.0.0.1", args.port), stub.server.RequestHandlerClass)
    stub.start()
    print(json.dumps({"ready": True, "port": stub.port}), flush=True)
    try:
        while True:
            time.sleep(3600)
    except KeyboardInterrupt:
        pass
    finally:
        stub.close()


if __name__ == "__main__":
    main()
