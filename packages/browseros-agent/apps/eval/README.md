# BrowserOS Eval

[![License: AGPL v3](https://img.shields.io/badge/License-AGPL%20v3-blue.svg)](../../../../LICENSE)

Evaluation framework for BrowserOS browser automation agents. Runs tasks from standard datasets ([WebVoyager](https://arxiv.org/abs/2401.13919), [Mind2Web](https://arxiv.org/abs/2306.06070), AGI SDK / REAL Bench, WebArena-Infinity, WebBench), captures trajectories with screenshots, and grades results automatically.

## Prerequisites

- **BrowserOS binary** at `/Applications/BrowserOS.app` (macOS) or `BROWSEROS_BINARY` pointing at it
- **Bun** runtime
- **API keys** for your LLM provider (and `CLAUDE_CODE_OAUTH_TOKEN` if you use `performance_grader`)

## Quick Start

```bash
cd apps/eval
# Edit .env.development with your keys, then:
bun run eval
```

Opens the eval dashboard at `http://localhost:9900` in config mode. From there: load a preset, edit settings, click **Run**.

### CLI mode

```bash
bun run eval -c configs/browseros-agent-weekly.json
```

Runs immediately. Dashboard still available at `http://localhost:9900` for live progress.

## Agent types

| Type | Description |
|------|-------------|
| `single` | Single LLM agent driven by the BrowserOS tool loop (CDP) |
| `orchestrator-executor` | High-level orchestrator + per-step executor (LLM or Clado visual model) |

### Single agent

```json
{
  "agent": {
    "type": "single",
    "provider": "openai-compatible",
    "model": "moonshotai/kimi-k2.5",
    "apiKey": "OPENROUTER_API_KEY",
    "baseUrl": "https://openrouter.ai/api/v1",
    "supportsImages": true
  }
}
```

### Orchestrator-Executor

The orchestrator works with any LLM provider. The executor can be another LLM, or the **Clado action** visual model that takes screenshots and predicts click/type/scroll coordinates.

```json
{
  "agent": {
    "type": "orchestrator-executor",
    "orchestrator": {
      "provider": "openai-compatible",
      "model": "accounts/fireworks/models/kimi-k2p5",
      "apiKey": "FIREWORKS_API_KEY",
      "baseUrl": "https://api.fireworks.ai/inference/v1"
    },
    "executor": {
      "provider": "clado-action",
      "model": "qwen3-vl-30b-a3b-instruct",
      "apiKey": "",
      "baseUrl": "https://clado-ai--clado-browseros-action-actionmodel-generate.modal.run"
    }
  }
}
```

## Graders

| Name | Description |
|------|-------------|
| `performance_grader` | Multi-axis grader running on Claude Agent SDK (uses its own credentials via `CLAUDE_CODE_OAUTH_TOKEN`) |
| `agisdk_state_diff` | AGI SDK / REAL Bench environment state-diff grader (deterministic) |
| `infinity_state` | WebArena-Infinity verifier-script grader (deterministic) |

Set `graders` in your config to override the per-task `graders` field from the dataset:

```json
"graders": ["performance_grader"]
```

## Configuration reference

### API keys

The `apiKey` field supports two formats:
- **Env var name**: `"OPENAI_API_KEY"` — resolved from `.env.development` at runtime
- **Direct value**: `"sk-xxxxx"` — used as-is (not recommended)

### Supported providers

| Provider | `provider` value | Requires `baseUrl` |
|----------|------------------|--------------------|
| OpenAI | `openai` | No |
| Anthropic | `anthropic` | No |
| Google | `google` | No |
| Azure OpenAI | `azure` | Yes |
| AWS Bedrock | `bedrock` | No |
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

Each worker gets its own Chrome instance. Worker N uses `base_port + N` for CDP and server ports.

### Execution settings

| Field | Description | Default |
|-------|-------------|---------|
| `num_workers` | Parallel workers (each gets its own Chrome) | `1` |
| `timeout_ms` | Per-task timeout in ms | `1800000` (30 min) |
| `restart_server_per_task` | Restart Chrome between tasks (cleaner state, slower) | `false` |

## Datasets

| File | Tasks | Description |
|------|-------|-------------|
| `webvoyager.jsonl` | 643 | Full WebVoyager benchmark |
| `mind2web.jsonl` | 300 | Online-Mind2Web |
| `webbench-{0,1,2}of4-50.jsonl` | 50 each | WebBench shards (50-task subsets) |
| `agisdk-real.jsonl` | 40 | AGI SDK / REAL Bench (action-only tasks) |
| `webarena-infinity-hard-50.jsonl` | 50 | WebArena-Infinity hard set |
| `browsecomp-medium-hard-50.jsonl` | 50 | BrowseComp medium-hard |
| `browsecomp-very-hard-50.jsonl` | 50 | BrowseComp very-hard |

Task format (JSONL, one per line):

```json
{
  "query_id": "Amazon--0",
  "dataset": "webvoyager",
  "query": "Search an Xbox Wireless controller with green color and rated above 4 stars.",
  "graders": ["performance_grader"],
  "start_url": "https://www.amazon.com/",
  "metadata": { "original_task_id": "Amazon--0", "website": "Amazon" }
}
```

## Output

Results are saved to `output_dir`:

```
results/
  browseros-agent-weekly/
    2026-04-29-1430/
      Amazon--0/
        metadata.json         # Task result, timing, grader scores
        messages.jsonl         # Full message log
        screenshots/
          001.png              # Step-by-step screenshots
          002.png
      summary.json             # Aggregate pass rates
```

## Troubleshooting

**BrowserOS not found**: Expects `/Applications/BrowserOS.app/Contents/MacOS/BrowserOS`. Set `BROWSEROS_BINARY` to override.

**Port conflicts**: Each worker uses `base_port + workerIndex`. 3 workers on base 9110 → ports 9110, 9111, 9112. Stop other BrowserOS instances first.

**API key not resolving**: If your config has `"apiKey": "OPENAI_API_KEY"`, ensure the env var is set in `.env.development`.

**Tasks timing out**: Increase `timeout_ms`. Default is 30 minutes.

**Headless vs headed**: Set `"headless": false` to watch Chrome in real time.
