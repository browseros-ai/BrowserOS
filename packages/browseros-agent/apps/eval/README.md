# BrowserOS Eval

[![License: AGPL v3](https://img.shields.io/badge/License-AGPL%20v3-blue.svg)](../../../../LICENSE)

Evaluation framework for benchmarking BrowserOS browser automation agents. Runs tasks from standard datasets ([WebVoyager](https://arxiv.org/abs/2401.13919), [Mind2Web](https://arxiv.org/abs/2306.06070)), captures trajectories with screenshots, and grades results automatically.

## Prerequisites

- **BrowserOS binary** installed at `/Applications/BrowserOS.app` (macOS)
- **Bun** runtime
- **API keys** for your chosen LLM provider and grader model

## Quick Start

### 1. Set up environment

```bash
cd apps/eval
```

Edit `.env.development` and add your API keys:

```bash
# Pick ONE provider for the orchestrator (whichever you have access to)
OPENAI_API_KEY=sk-xxxxx
ANTHROPIC_API_KEY=sk-ant-xxxxx
FIREWORKS_API_KEY=fw_xxxxx
GOOGLE_API_KEY=AIza-xxxxx

# For grading results (OpenRouter recommended — gives access to many models)
OPENROUTER_API_KEY=sk-or-v1-xxxxx
```

### 2. Launch the dashboard

```bash
bun run eval
```

Opens the **Eval Dashboard** at `http://localhost:9900` in config mode.

### 3. Configure and run

From the dashboard:

1. **Load a preset** — select from the dropdown or click **Load File** to import a config JSON
2. **Edit settings** — change agent type, provider, model, API keys, dataset, workers, timeouts
3. **Save Config** — export your configuration for reuse
4. **Click Run** — starts the evaluation with live progress

### Alternative: Run from CLI

```bash
bun run eval -c configs/orchestrator-executor-clado-test.json
```

Runs immediately. Dashboard still available at `http://localhost:9900` for live progress.

## Agent Types

### Orchestrator-Executor with Clado

The recommended architecture for visual model evals. Two tiers:

- **Orchestrator** — An LLM that plans and issues high-level instructions
- **Executor** — The **Clado Action** visual model that takes screenshots and predicts click/type/scroll coordinates

The orchestrator works with **any LLM provider**. Pick whichever you have access to:

#### OpenAI orchestrator

```json
{
  "agent": {
    "type": "orchestrator-executor",
    "orchestrator": {
      "provider": "openai",
      "model": "gpt-4o",
      "apiKey": "OPENAI_API_KEY"
    },
    "executor": {
      "provider": "clado-action",
      "model": "qwen3-vl-30b-a3b-instruct",
      "apiKey": "",
      "baseUrl": "https://clado-ai--clado-browseros-action-actionmodel-generate.modal.run"
    }
  },
  "dataset": "../data/webvoyager_e2e_test.jsonl",
  "output_dir": "../results/oe-clado-openai",
  "num_workers": 3,
  "browseros": {
    "server_url": "http://127.0.0.1:9110",
    "base_cdp_port": 9010,
    "base_server_port": 9110,
    "base_extension_port": 9310,
    "headless": true
  },
  "grader_api_key_env": "OPENROUTER_API_KEY",
  "grader_base_url": "https://openrouter.ai/api/v1",
  "grader_model": "openai/gpt-4.1",
  "timeout_ms": 1200000
}
```

#### Anthropic orchestrator

```json
"orchestrator": {
  "provider": "anthropic",
  "model": "claude-sonnet-4-20250514",
  "apiKey": "ANTHROPIC_API_KEY"
}
```

#### Google orchestrator

```json
"orchestrator": {
  "provider": "google",
  "model": "gemini-2.0-flash",
  "apiKey": "GOOGLE_API_KEY"
}
```

#### Fireworks orchestrator (OpenAI-compatible)

```json
"orchestrator": {
  "provider": "openai-compatible",
  "model": "accounts/fireworks/models/kimi-k2p5",
  "apiKey": "FIREWORKS_API_KEY",
  "baseUrl": "https://api.fireworks.ai/inference/v1"
}
```

The executor config stays the same across all orchestrator providers — it always uses the Clado action model.

### Other Agent Types

| Type | Description | Example config |
|------|-------------|----------------|
| `single` | Single LLM agent via Gemini CLI + MCP | `webvoyager-test.json` |
| `tool-loop` | AI SDK tool loop, connects via CDP | `tool-loop-test.json` |
| `gemini-computer-use` | Google native computer use API | `gemini-computer-use.json` |
| `yutori-navigator` | Yutori N1 visual model | `yutori-navigator.json` |

## Configuration Reference

### API keys

The `apiKey` field supports two formats:
- **Env var name**: `"OPENAI_API_KEY"` — resolved from `.env.development` at runtime
- **Direct value**: `"sk-xxxxx"` — used as-is (not recommended, prefer env vars)

### Supported providers

| Provider | `provider` value | Requires `baseUrl` |
|----------|------------------|--------------------|
| OpenAI | `openai` | No |
| Anthropic | `anthropic` | No |
| Google | `google` | No |
| Azure OpenAI | `azure` | Yes |
| AWS Bedrock | `bedrock` | No (uses `region`, `accessKeyId`, `secretAccessKey`) |
| OpenRouter | `openrouter` | No |
| Fireworks, Together, etc. | `openai-compatible` | Yes |
| Ollama | `ollama` | No |
| Clado Action (executor only) | `clado-action` | Yes |

### BrowserOS infrastructure

```json
"browseros": {
  "server_url": "http://127.0.0.1:9110",
  "base_cdp_port": 9010,
  "base_server_port": 9110,
  "base_extension_port": 9310,
  "load_extensions": false,
  "headless": true
}
```

Each worker gets its own Chrome instance. Worker N uses `base_port + N` for CDP and server ports. `base_extension_port` is still reserved as a legacy BrowserOS launch argument for compatibility with Chromium builds that still pass it.

### Execution settings

| Field | Description | Default |
|-------|-------------|---------|
| `num_workers` | Parallel workers (each gets its own Chrome) | `1` |
| `timeout_ms` | Per-task timeout in ms | `900000` (15 min) |
| `restart_server_per_task` | Restart Chrome between tasks (cleaner state, slower) | `false` |

### Grading

Results are auto-graded after each task. The grader uses an LLM judge.

| Field | Description |
|-------|-------------|
| `grader_model` | Model for grading (e.g., `openai/gpt-4.1`) |
| `grader_api_key_env` | Env var name for grader API key |
| `grader_base_url` | API endpoint (e.g., `https://openrouter.ai/api/v1`) |

## Datasets

| File | Tasks | Description |
|------|-------|-------------|
| `webvoyager_e2e_test.jsonl` | 10 | WebVoyager test subset (quick smoke test) |
| `webvoyager.jsonl` | 643 | Full WebVoyager benchmark |
| `mind2web_e2e_test.jsonl` | 10 | Mind2Web test subset |
| `mind2web.jsonl` | 300 | Full Mind2Web benchmark |

Task format (JSONL, one per line):

```json
{
  "query_id": "Amazon--0",
  "dataset": "webvoyager",
  "query": "Search an Xbox Wireless controller with green color and rated above 4 stars.",
  "graders": ["webvoyager_grader", "fara_combined"],
  "start_url": "https://www.amazon.com/",
  "metadata": { "original_task_id": "Amazon--0", "website": "Amazon" }
}
```

## Output

Results are saved to `output_dir`:

```
results/
  oe-clado-openai/
    Amazon--0/
      metadata.json         # Task result, timing, grader scores
      messages.jsonl         # Full message log
      screenshots/
        001.png              # Step-by-step screenshots
        002.png
    summary.json             # Aggregate pass rates
```

## Troubleshooting

**BrowserOS not found**: Expects `/Applications/BrowserOS.app/Contents/MacOS/BrowserOS`. Make sure it's installed.

**Port conflicts**: Each worker uses `base_port + workerIndex`. 3 workers on base 9110 → ports 9110, 9111, 9112. Stop other BrowserOS instances first.

**API key not resolving**: If your config has `"apiKey": "OPENAI_API_KEY"`, ensure the env var is set in `.env.development`.

**Tasks timing out**: Increase `timeout_ms`. Default is 15 minutes; complex tasks may need 20+ minutes.

**Headless vs headed**: Set `"headless": false` to watch Chrome in real-time. Useful for debugging.
