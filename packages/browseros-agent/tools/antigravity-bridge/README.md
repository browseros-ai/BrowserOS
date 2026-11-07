# Antigravity bridge

A tiny, dependency-free (stdlib only, plus the SDK) HTTP server that exposes the
[Google Antigravity Python SDK](https://github.com/google-antigravity/antigravity-sdk-python)
as an OpenAI-compatible endpoint, so BrowserOS's **Antigravity** LLM provider
(`antigravity`, served via `@ai-sdk/openai-compatible`) can use it.

Developed against SDK commit `75ba22bd00afae2f46393ff31f30c3f0df52982f`
(`google-antigravity` 0.1.23 on PyPI).

## Setup

```bash
cd packages/browseros-agent/tools/antigravity-bridge
python -m venv .venv && . .venv/bin/activate
pip install -r requirements.txt   # installs google-antigravity (includes runtime binary)
```

> Install the SDK from PyPI; a git clone of the SDK lacks the compiled runtime.

## Auth

Uses whatever the SDK uses:

- Gemini API: `export GEMINI_API_KEY=...`
- Vertex AI: `GOOGLE_GENAI_USE_VERTEXAI=True`, `GOOGLE_CLOUD_PROJECT`,
  `GOOGLE_CLOUD_LOCATION`, and `gcloud auth application-default login`.

Optional: `ANTIGRAVITY_BRIDGE_TOKEN=...` makes the bridge require
`Authorization: Bearer <token>`; put the same value in the provider's API Key.

## Run

```bash
python antigravity_bridge.py --host 127.0.0.1 --port 8765 \
  --models gemini-3.8-flash          # or ANTIGRAVITY_BRIDGE_MODELS
```

Endpoints: `GET /v1/models`, `POST /v1/chat/completions` (`stream: true` → SSE).

## BrowserOS config

Settings → AI → add **Antigravity**: Base URL `http://127.0.0.1:8765/v1`,
Model ID `gemini-3.8-flash`, API Key empty (or the bridge token).

## Limitations

- Each request is one stateless `Agent.chat` turn; prior messages are flattened
  into the prompt, and a fresh SDK agent/runtime is started per request (latency).
- Antigravity builtin tools and subagents are disabled (`enabled_tools=[]`).
- OpenAI `tools`/function calling, `tool` messages and non-text content parts are
  rejected with HTTP 400, so BrowserOS agent mode (browser tool calls) does not
  work with this provider — use it for chat-style prompts.
- No usage/token accounting; sampling params (temperature etc.) are ignored.
- Binds to localhost by default; do not expose it publicly.

## Tests

```bash
pip install -r requirements-dev.txt
python -m pytest tests
```
