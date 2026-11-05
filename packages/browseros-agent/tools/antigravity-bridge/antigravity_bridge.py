"""OpenAI-compatible HTTP bridge for the Google Antigravity Python SDK.

Exposes ``GET /v1/models`` and ``POST /v1/chat/completions`` (JSON and SSE
streaming) on localhost so BrowserOS's "Antigravity" provider, which speaks the
OpenAI Chat Completions protocol via ``@ai-sdk/openai-compatible``, can drive
Antigravity / Gemini models.

Each request runs one stateless turn: the OpenAI ``messages`` array is
flattened into a single prompt and sent with ``Agent.chat``. Antigravity's
builtin tools and subagents are disabled so the bridge behaves like a plain
model endpoint. OpenAI ``tools`` / function calling is NOT supported.

Auth is whatever the SDK uses: ``GEMINI_API_KEY`` or Vertex env vars
(``GOOGLE_GENAI_USE_VERTEXAI``, ``GOOGLE_CLOUD_PROJECT``,
``GOOGLE_CLOUD_LOCATION`` + ADC). Optionally set ``ANTIGRAVITY_BRIDGE_TOKEN``
to require ``Authorization: Bearer <token>`` from callers.

Usage::

    pip install google-antigravity
    export GEMINI_API_KEY=...
    python antigravity_bridge.py --port 8765
"""

from __future__ import annotations

import argparse
import asyncio
import json
import os
import time
import uuid
from collections.abc import AsyncIterator, Callable
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import Any

DEFAULT_MODEL = "gemini-3.8-flash"
DEFAULT_HOST = "127.0.0.1"
DEFAULT_PORT = 8765

# A stream factory takes (model, system, prompt) and yields text deltas.
StreamFactory = Callable[[str, str | None, str], AsyncIterator[str]]


class BridgeError(Exception):
  def __init__(self, status: int, message: str):
    super().__init__(message)
    self.status = status
    self.message = message


def _content_to_text(content: Any) -> str:
  """Converts an OpenAI message content (str or parts list) to text."""
  if content is None:
    return ""
  if isinstance(content, str):
    return content
  if isinstance(content, list):
    texts = []
    for part in content:
      if isinstance(part, dict) and part.get("type") == "text":
        texts.append(str(part.get("text", "")))
      elif isinstance(part, dict):
        raise BridgeError(
            400, f"Unsupported content part type: {part.get('type')!r}"
        )
    return "\n".join(texts)
  raise BridgeError(400, "Invalid message content")


def messages_to_prompt(messages: Any) -> tuple[str | None, str]:
  """Flattens OpenAI chat messages into (system_instructions, prompt)."""
  if not isinstance(messages, list) or not messages:
    raise BridgeError(400, "'messages' must be a non-empty array")
  system_parts: list[str] = []
  turns: list[tuple[str, str]] = []
  for msg in messages:
    if not isinstance(msg, dict):
      raise BridgeError(400, "Each message must be an object")
    role = msg.get("role")
    text = _content_to_text(msg.get("content"))
    if role in ("system", "developer"):
      system_parts.append(text)
    elif role in ("user", "assistant"):
      turns.append((role, text))
    elif role == "tool":
      raise BridgeError(400, "Tool messages are not supported by this bridge")
    else:
      raise BridgeError(400, f"Unsupported role: {role!r}")
  if not turns:
    raise BridgeError(400, "At least one user message is required")
  system = "\n\n".join(p for p in system_parts if p) or None
  if len(turns) == 1 and turns[0][0] == "user":
    return system, turns[0][1]
  lines = ["Conversation so far:"]
  for role, text in turns[:-1]:
    lines.append(f"{role.upper()}: {text}")
  last_role, last_text = turns[-1]
  lines.append(f"{last_role.upper()}: {last_text}")
  lines.append("Respond as the ASSISTANT to the last message.")
  return system, "\n\n".join(lines)


async def antigravity_stream(
    model: str, system: str | None, prompt: str
) -> AsyncIterator[str]:
  """Default stream factory backed by the real Antigravity SDK."""
  # Imported lazily so tests and --help work without the SDK installed.
  from google.antigravity import Agent, CapabilitiesConfig, LocalAgentConfig

  config = LocalAgentConfig(
      model=model,
      system_instructions=system,
      capabilities=CapabilitiesConfig(
          enabled_tools=[], enable_subagents=False
      ),
  )
  async with Agent(config) as agent:
    response = await agent.chat(prompt)
    async for token in response:
      yield token


def _chunk(cid: str, model: str, delta: dict, finish: str | None) -> dict:
  return {
      "id": cid,
      "object": "chat.completion.chunk",
      "created": int(time.time()),
      "model": model,
      "choices": [{"index": 0, "delta": delta, "finish_reason": finish}],
  }


def make_handler(
    stream_factory: StreamFactory = antigravity_stream,
    models: list[str] | None = None,
    token: str | None = None,
) -> type[BaseHTTPRequestHandler]:
  model_ids = models or [DEFAULT_MODEL]

  class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, fmt, *args):  # quieter default logging
      if os.environ.get("ANTIGRAVITY_BRIDGE_DEBUG"):
        super().log_message(fmt, *args)

    def _json(self, status: int, body: dict) -> None:
      data = json.dumps(body).encode()
      self.send_response(status)
      self.send_header("Content-Type", "application/json")
      self.send_header("Content-Length", str(len(data)))
      self.end_headers()
      self.wfile.write(data)

    def _error(self, status: int, message: str) -> None:
      self._json(
          status, {"error": {"message": message, "type": "invalid_request_error"}}
      )

    def _authorized(self) -> bool:
      if not token:
        return True
      return self.headers.get("Authorization") == f"Bearer {token}"

    def do_GET(self):  # noqa: N802
      if not self._authorized():
        return self._error(401, "Unauthorized")
      if self.path.rstrip("/") in ("/v1/models", "/models"):
        return self._json(
            200,
            {
                "object": "list",
                "data": [
                    {"id": m, "object": "model", "owned_by": "antigravity"}
                    for m in model_ids
                ],
            },
        )
      return self._error(404, "Not found")

    def do_POST(self):  # noqa: N802
      if not self._authorized():
        return self._error(401, "Unauthorized")
      if self.path.rstrip("/") not in (
          "/v1/chat/completions",
          "/chat/completions",
      ):
        return self._error(404, "Not found")
      try:
        length = int(self.headers.get("Content-Length") or 0)
        body = json.loads(self.rfile.read(length) or b"{}")
        if not isinstance(body, dict):
          raise BridgeError(400, "Request body must be a JSON object")
        if body.get("tools"):
          raise BridgeError(
              400, "Tool/function calling is not supported by this bridge"
          )
        model = body.get("model") or model_ids[0]
        system, prompt = messages_to_prompt(body.get("messages"))
      except json.JSONDecodeError:
        return self._error(400, "Invalid JSON")
      except BridgeError as e:
        return self._error(e.status, e.message)

      cid = f"chatcmpl-{uuid.uuid4().hex}"
      if body.get("stream"):
        return self._stream(cid, model, system, prompt)
      try:
        text = asyncio.run(self._collect(model, system, prompt))
      except Exception as e:  # pylint: disable=broad-except
        return self._json(502, {"error": {"message": str(e), "type": "upstream_error"}})
      return self._json(
          200,
          {
              "id": cid,
              "object": "chat.completion",
              "created": int(time.time()),
              "model": model,
              "choices": [
                  {
                      "index": 0,
                      "message": {"role": "assistant", "content": text},
                      "finish_reason": "stop",
                  }
              ],
          },
      )

    async def _collect(self, model, system, prompt) -> str:
      return "".join([t async for t in stream_factory(model, system, prompt)])

    def _send_event(self, payload: dict | str) -> None:
      data = payload if isinstance(payload, str) else json.dumps(payload)
      line = f"data: {data}\n\n".encode()
      self.wfile.write(f"{len(line):x}\r\n".encode() + line + b"\r\n")
      self.wfile.flush()

    def _stream(self, cid, model, system, prompt) -> None:
      self.send_response(200)
      self.send_header("Content-Type", "text/event-stream")
      self.send_header("Cache-Control", "no-cache")
      self.send_header("Transfer-Encoding", "chunked")
      self.end_headers()

      async def run():
        self._send_event(_chunk(cid, model, {"role": "assistant"}, None))
        async for token in stream_factory(model, system, prompt):
          if token:
            self._send_event(_chunk(cid, model, {"content": token}, None))

      try:
        asyncio.run(run())
        self._send_event(_chunk(cid, model, {}, "stop"))
      except Exception as e:  # pylint: disable=broad-except
        self._send_event({"error": {"message": str(e), "type": "upstream_error"}})
      self._send_event("[DONE]")
      self.wfile.write(b"0\r\n\r\n")
      self.wfile.flush()

  return Handler


def main(argv: list[str] | None = None) -> None:
  parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
  parser.add_argument("--host", default=DEFAULT_HOST)
  parser.add_argument("--port", type=int, default=DEFAULT_PORT)
  parser.add_argument(
      "--models",
      default=os.environ.get("ANTIGRAVITY_BRIDGE_MODELS", DEFAULT_MODEL),
      help="Comma-separated model ids advertised on /v1/models.",
  )
  args = parser.parse_args(argv)
  handler = make_handler(
      models=[m.strip() for m in args.models.split(",") if m.strip()],
      token=os.environ.get("ANTIGRAVITY_BRIDGE_TOKEN") or None,
  )
  server = ThreadingHTTPServer((args.host, args.port), handler)
  print(f"Antigravity bridge listening on http://{args.host}:{args.port}/v1")
  try:
    server.serve_forever()
  except KeyboardInterrupt:
    pass
  finally:
    server.server_close()


if __name__ == "__main__":
  main()
