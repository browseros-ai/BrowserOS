# BrowserOS Eval

[![License: AGPL v3](https://img.shields.io/badge/License-AGPL%20v3-blue.svg)](../../../../LICENSE)

Evaluation framework for BrowserOS browser automation agents. Runs tasks from standard datasets ([WebVoyager](https://arxiv.org/abs/2401.13919), [Mind2Web](https://arxiv.org/abs/2306.06070), AGI SDK / REAL Bench, WebArena-Infinity, WebBench), captures trajectories with screenshots, and grades results automatically.

## Prerequisites

- **BrowserOS binary** at `/Applications/BrowserOS.app` (macOS) or `BROWSEROS_BINARY` pointing at it
- **Bun** runtime
- **API keys** for your LLM provider (and `CLAUDE_CODE_OAUTH_TOKEN` if you use `performance_grader`)
- **Python 3.10+ with `agisdk`** for AGI SDK / REAL Bench grading. Set `BROWSEROS_EVAL_PYTHON` if your default `python3` is older.

## Quick Start

```bash
cd apps/eval
(cd ../.. && cp .env.development.example .env.development)
# Edit ../../.env.development with your keys, then:
bun run eval
```

Opens the eval dashboard at `http://localhost:9900` in config mode. From there: load a preset, edit settings, click **Run**.

### CLI mode

```bash
bun run eval -c configs/legacy/browseros-agent-weekly.json
bun run eval suite --config configs/legacy/browseros-agent-weekly.json --publish r2
```

Runs immediately. Dashboard still available at `http://localhost:9900` for live progress.

The `suite` command is the workflow-compatible full loop: execute tasks, run graders, write artifacts, and optionally publish to R2. The old `-c` form remains supported during migration.

```bash
bun run eval run --config configs/legacy/browseros-agent-weekly.json
bun run eval suite --suite configs/suites/agisdk-daily-10.json --variant kimi-fireworks --publish r2
bun run eval grade --run results/browseros-agent-weekly/2026-04-29-1430
bun run eval publish --run results/browseros-agent-weekly/2026-04-29-1430 --target r2
```

Config files live in two groups:

```txt
configs/legacy/  # Complete EvalConfig files used by older workflows and the dashboard
configs/suites/  # Suite definitions; model/provider comes from CLI flags or env
```

Suite mode takes model settings from CLI flags first, then env:

```bash
EVAL_VARIANT=kimi-fireworks \
EVAL_AGENT_PROVIDER=openai-compatible \
EVAL_AGENT_MODEL=accounts/fireworks/models/kimi-k2p5 \
EVAL_AGENT_API_KEY=$FIREWORKS_API_KEY \
EVAL_AGENT_BASE_URL=https://api.fireworks.ai/inference/v1 \
bun run eval suite --suite configs/suites/agisdk-daily-10.json --publish r2
```

### Suites and variants

A **suite** is what we run: the task dataset, graders, worker count, timeout, and browser settings. For example, `agisdk-daily-10` means "run these 10 AGI SDK tasks and grade them with `agisdk_state_diff`."

A **variant** is the model setup we are testing on that suite. `EVAL_VARIANT` is just the human-readable name for that setup. The actual model connection still comes from `EVAL_AGENT_PROVIDER`, `EVAL_AGENT_MODEL`, `EVAL_AGENT_API_KEY`, and `EVAL_AGENT_BASE_URL`.

This lets us run the same suite against multiple model setups without copying the benchmark config:

```txt
agisdk-daily-10 + kimi-fireworks
agisdk-daily-10 + claude-opus
agisdk-daily-10 + clado-action-000159
```

For `orchestrator-executor` suites, there can also be an executor model/backend. The `EVAL_AGENT_*` vars describe the main agent or orchestrator. The optional `EVAL_EXECUTOR_*` or `CLADO_ACTION_*` vars describe the delegated executor.

## Agent types

| Type | Description |
|------|-------------|
| `single` | Single LLM agent driven by the BrowserOS tool loop (CDP) |
| `orchestrator-executor` | High-level orchestrator + per-step executor (LLM or Clado visual model) |
| `claude-code` | External Claude Code CLI driven through BrowserOS MCP |

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
      "model": "Qwen3.5-35B-A3B-action-000159-merged",
      "apiKey": "",
      "baseUrl": "https://clado-ai--clado-browseros-action-000159-merged-actionmod-f4a6ef.modal.run"
    }
  }
}
```

### Claude Code

Claude Code runs as an external `claude -p` subprocess. The eval runner passes a task-scoped MCP config that points Claude Code at the active worker's BrowserOS MCP endpoint, while the eval capture layer still saves messages, screenshots, trajectory metadata, and grader outputs.

```json
{
  "agent": {
    "type": "claude-code",
    "model": "opus"
  }
}
```

```bash
BROWSEROS_EVAL_PYTHON=/path/to/python3 bun run eval run --config configs/legacy/claude-code-agisdk-real.json
bun run eval suite --config configs/legacy/claude-code-agisdk-real.json --publish r2
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
- **Env var name**: `"OPENAI_API_KEY"` — resolved from the monorepo root `.env.development` at runtime
- **Direct value**: `"sk-xxxxx"` — used as-is (not recommended)

### Environment variables

Set eval environment variables in the monorepo root `.env.development` file.

| Variable | Used for |
|----------|----------|
| `EVAL_AGENT_PROVIDER`, `EVAL_AGENT_MODEL`, `EVAL_AGENT_API_KEY`, `EVAL_AGENT_BASE_URL`, `EVAL_AGENT_SUPPORTS_IMAGES` | Suite variant model selection |
| `FIREWORKS_API_KEY`, `OPENROUTER_API_KEY`, `ANTHROPIC_API_KEY`, provider-specific keys | Config-file or provider-backed model calls |
| `EVAL_EXECUTOR_MODEL`, `EVAL_EXECUTOR_API_KEY`, `EVAL_EXECUTOR_BASE_URL` | Suite-mode orchestrator executor override |
| `CLADO_ACTION_MODEL`, `CLADO_ACTION_API_KEY`, `CLADO_ACTION_BASE_URL` | Clado executor defaults |
| `BROWSEROS_BINARY` | BrowserOS binary path in CI/local smoke runs |
| `BROWSEROS_SERVER_URL` | Optional grader MCP URL override |
| `BROWSEROS_EVAL_PYTHON` | Optional Python interpreter for JSON graders such as `agisdk_state_diff` |
| `WEBARENA_INFINITY_DIR` | Local WebArena-Infinity checkout for Infinity tasks |
| `NOPECHA_API_KEY` | CAPTCHA solver extension |
| `EVAL_R2_ACCOUNT_ID`, `EVAL_R2_ACCESS_KEY_ID`, `EVAL_R2_SECRET_ACCESS_KEY`, `EVAL_R2_BUCKET`, `EVAL_R2_CDN_BASE_URL` | R2 upload and viewer URL |

### Supported providers

| Provider | `provider` value | Requires `baseUrl` |
|----------|------------------|--------------------|
| OpenAI | `openai` | No |
| Anthropic | `anthropic` | No |
| Google | `google` | No |
| Azure OpenAI | `azure` | Yes |
| AWS Bedrock | `bedrock` | No |
| OpenRouter | `openrouter` | No |
| Requesty | `requesty` | No |
| Fireworks, Together, etc. | `openai-compatible` | Yes |
| Ollama | `ollama` | No |
| Clado Action (executor only) | `clado-action` | Yes |

### R2 publishing

`suite --config ... --publish r2` and `publish --target r2` upload the run artifacts plus `viewer.html` to the viewer-compatible R2 layout:

```bash
export EVAL_R2_ACCOUNT_ID=...
export EVAL_R2_ACCESS_KEY_ID=...
export EVAL_R2_SECRET_ACCESS_KEY=...
export EVAL_R2_BUCKET=browseros-eval
export EVAL_R2_CDN_BASE_URL=https://eval.browseros.com
```

`EVAL_R2_CDN_BASE_URL` must be a public R2 custom domain, `r2.dev` URL, or Worker URL. Do not set it to the private `*.r2.cloudflarestorage.com` S3 API endpoint.

Published runs are available at `EVAL_R2_CDN_BASE_URL/viewer.html?run=<run-id>`.

### BrowserOS infrastructure

```json
"browseros": {
  "server_url": "http://127.0.0.1:9110",
  "base_cdp_port": 9010,
  "base_server_port": 9110,
  "base_extension_port": 9310,
  "load_extensions": false,
  "headless": false
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
| `agisdk-daily-10.jsonl` | 10 | Daily AGI SDK / REAL Bench subset |
| `webvoyager.jsonl` | 643 | Full WebVoyager benchmark |
| `mind2web.jsonl` | 300 | Online-Mind2Web |
| `webbench-{0,1,2}of4-50.jsonl` | 50 each | WebBench shards (50-task subsets) |
| `agisdk-real-smoke.jsonl` | 1 | AGI SDK / REAL Bench smoke task |
| `agisdk-real.jsonl` | 36 | AGI SDK / REAL Bench (action-only tasks) |
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
        attempt.json          # Stable attempt summary for viewer/reporting
        metadata.json         # Task result, timing, grader scores
        grades.json           # Compact grader results
        messages.jsonl         # Full message log
        grader-artifacts/      # Grader-specific inputs/outputs/stderr
        screenshots/
          001.png              # Step-by-step screenshots
          002.png
      summary.json             # Aggregate pass rates
```

R2 publishing preserves the task files under `runs/<run-id>/...`, writes `runs/<run-id>/manifest.json`, and uploads `viewer.html` at the bucket root. The viewer URL is `EVAL_R2_CDN_BASE_URL/viewer.html?run=<run-id>`.

### R2 viewer manifest

`runs/<run-id>/manifest.json` is the source of truth for the public viewer. New manifests include `schemaVersion: 2` and each task includes explicit artifact paths:

```json
{
  "schemaVersion": 2,
  "runId": "agisdk-real-smoke-2026-04-30-0000",
  "tasks": [
    {
      "queryId": "agisdk-dashdish-10",
      "paths": {
        "metadata": "tasks/agisdk-dashdish-10/metadata.json",
        "messages": "tasks/agisdk-dashdish-10/messages.jsonl",
        "grades": "tasks/agisdk-dashdish-10/grades.json",
        "trace": "tasks/agisdk-dashdish-10/trace.jsonl",
        "screenshots": "tasks/agisdk-dashdish-10/screenshots",
        "graderArtifacts": "tasks/agisdk-dashdish-10/grader-artifacts"
      }
    }
  ]
}
```

The static viewer uses `task.paths` when present. Older uploaded runs without `schemaVersion` or `task.paths` still work through the legacy inferred layout: `runs/<run-id>/<task-id>/metadata.json`, `messages.jsonl`, and `screenshots/<n>.png`.

Manifest paths are stable artifact locations, not a guarantee that every optional artifact exists for every task. For example, `attempt.json`, `trace.jsonl`, or grader artifact directories may be absent when that artifact was not produced by the run.

## Troubleshooting

**BrowserOS not found**: Expects `/Applications/BrowserOS.app/Contents/MacOS/BrowserOS`. Set `BROWSEROS_BINARY` to override.

**Port conflicts**: Each worker uses `base_port + workerIndex`. 3 workers on base 9110 → ports 9110, 9111, 9112. Stop other BrowserOS instances first.

**API key not resolving**: If your config has `"apiKey": "OPENAI_API_KEY"`, ensure the env var is set in the monorepo root `.env.development`.

**Tasks timing out**: Increase `timeout_ms`. Default is 30 minutes.

**Headless vs headed**: Set `"headless": false` to watch Chrome in real time.
