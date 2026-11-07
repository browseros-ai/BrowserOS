import json
import pathlib
import sys
import threading
import urllib.error
import urllib.request
from http.server import ThreadingHTTPServer

import pytest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1]))

import antigravity_bridge as bridge  # noqa: E402

calls = []


async def fake_stream(model, system, prompt):
  calls.append((model, system, prompt))
  for t in ["Hello", ", ", "world"]:
    yield t


async def failing_stream(model, system, prompt):
  raise RuntimeError("boom")
  yield ""  # pragma: no cover


@pytest.fixture
def server(request):
  factory = getattr(request, "param", {}).get("factory", fake_stream)
  token = getattr(request, "param", {}).get("token")
  calls.clear()
  srv = ThreadingHTTPServer(
      ("127.0.0.1", 0),
      bridge.make_handler(factory, models=["gemini-a", "gemini-b"], token=token),
  )
  t = threading.Thread(target=srv.serve_forever, daemon=True)
  t.start()
  yield f"http://127.0.0.1:{srv.server_address[1]}"
  srv.shutdown()
  srv.server_close()


def _req(url, body=None, headers=None):
  data = json.dumps(body).encode() if body is not None else None
  req = urllib.request.Request(url, data=data, headers=headers or {})
  if data:
    req.add_header("Content-Type", "application/json")
  try:
    with urllib.request.urlopen(req) as r:
      return r.status, r.read().decode()
  except urllib.error.HTTPError as e:
    return e.code, e.read().decode()


def test_messages_single_user():
  assert bridge.messages_to_prompt([{"role": "user", "content": "hi"}]) == (
      None,
      "hi",
  )


def test_messages_system_and_history():
  system, prompt = bridge.messages_to_prompt([
      {"role": "system", "content": "be brief"},
      {"role": "user", "content": [{"type": "text", "text": "q1"}]},
      {"role": "assistant", "content": "a1"},
      {"role": "user", "content": "q2"},
  ])
  assert system == "be brief"
  assert "USER: q1" in prompt and "ASSISTANT: a1" in prompt
  assert prompt.index("ASSISTANT: a1") < prompt.index("USER: q2")


@pytest.mark.parametrize(
    "messages",
    [
        [],
        "x",
        [{"role": "system", "content": "only"}],
        [{"role": "tool", "content": "x"}],
        [{"role": "user", "content": [{"type": "image_url"}]}],
    ],
)
def test_messages_invalid(messages):
  with pytest.raises(bridge.BridgeError):
    bridge.messages_to_prompt(messages)


def test_models(server):
  status, body = _req(server + "/v1/models")
  assert status == 200
  assert [m["id"] for m in json.loads(body)["data"]] == ["gemini-a", "gemini-b"]


def test_chat_completion(server):
  status, body = _req(
      server + "/v1/chat/completions",
      {"model": "gemini-b", "messages": [{"role": "user", "content": "hi"}]},
  )
  assert status == 200
  data = json.loads(body)
  assert data["object"] == "chat.completion"
  assert data["choices"][0]["message"] == {
      "role": "assistant",
      "content": "Hello, world",
  }
  assert calls == [("gemini-b", None, "hi")]


def test_default_model(server):
  _req(server + "/v1/chat/completions", {"messages": [{"role": "user", "content": "x"}]})
  assert calls[0][0] == "gemini-a"


def test_streaming(server):
  status, body = _req(
      server + "/v1/chat/completions",
      {"stream": True, "messages": [{"role": "user", "content": "hi"}]},
  )
  assert status == 200
  events = [l[6:] for l in body.splitlines() if l.startswith("data: ")]
  assert events[-1] == "[DONE]"
  chunks = [json.loads(e) for e in events[:-1]]
  assert chunks[0]["choices"][0]["delta"] == {"role": "assistant"}
  text = "".join(c["choices"][0]["delta"].get("content", "") for c in chunks)
  assert text == "Hello, world"
  assert chunks[-1]["choices"][0]["finish_reason"] == "stop"


def test_tools_rejected(server):
  status, body = _req(
      server + "/v1/chat/completions",
      {"tools": [{"type": "function"}], "messages": [{"role": "user", "content": "x"}]},
  )
  assert status == 400
  assert "not supported" in json.loads(body)["error"]["message"]


def test_bad_json_and_404(server):
  req = urllib.request.Request(
      server + "/v1/chat/completions", data=b"{nope", method="POST"
  )
  with pytest.raises(urllib.error.HTTPError) as e:
    urllib.request.urlopen(req)
  assert e.value.code == 400
  assert _req(server + "/nope")[0] == 404


@pytest.mark.parametrize("server", [{"factory": failing_stream}], indirect=True)
def test_upstream_error(server):
  status, body = _req(
      server + "/v1/chat/completions",
      {"messages": [{"role": "user", "content": "x"}]},
  )
  assert status == 502
  assert "boom" in body


@pytest.mark.parametrize("server", [{"token": "secret"}], indirect=True)
def test_token_auth(server):
  assert _req(server + "/v1/models")[0] == 401
  assert (
      _req(server + "/v1/models", headers={"Authorization": "Bearer secret"})[0]
      == 200
  )
