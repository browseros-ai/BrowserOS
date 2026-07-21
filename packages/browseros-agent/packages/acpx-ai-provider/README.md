# acpx-ai-provider

> Vercel AI SDK provider on top of [`acpx/runtime`](https://www.npmjs.com/package/acpx).
> One install, any ACP agent — Claude Code, Codex, Gemini, Copilot, Cursor, Pi, and more.

[![npm](https://img.shields.io/npm/v/acpx-ai-provider.svg)](https://www.npmjs.com/package/acpx-ai-provider)

> [!WARNING]
> **Alpha software.** Both this package and its underlying runtime
> ([`acpx`](https://www.npmjs.com/package/acpx)) are pre-1.0. Public
> APIs may change in any minor release. Pin a version in production
> and read the [Known limitations](#known-limitations) section before
> picking it up — most of the rough edges flow through from
> `acpx/runtime`, which is itself still stabilizing its event shape.

## Why

The existing [`acp-ai-provider`](https://github.com/mcpc-tech/mcpc/tree/main/packages/acp-ai-provider)
bridges Vercel AI SDK to the Agent Client Protocol via the bare
`@agentclientprotocol/sdk`. That works, but consumers still have to
install each agent's CLI and write their own `{ command, args }`
spawn config.

`acpx-ai-provider` sits one level higher — on top of `acpx/runtime` — so:

- **Zero extra installs.** `acpx` resolves and `npx`-spawns built-in
  agents (Claude, Codex, Gemini, Copilot, Cursor, Pi, etc.) on first
  use.
- **No stdio plumbing, no init handshake, no auth retry loop, no
  permission dialog wiring** — the runtime owns all of that.
- The provider is a thin translation layer between AI SDK's
  `LanguageModelV2` and `AcpRuntime`'s normalized event stream.

## Install

```bash
bun add acpx-ai-provider acpx ai
# or
npm i acpx-ai-provider acpx ai
```

`acpx` and `ai` are peer dependencies. Use `ai` ≥ 6, `acpx` ≥ 0.6.

## Quickstart

```ts
import { createAcpxProvider } from 'acpx-ai-provider'
import { generateText } from 'ai'

const provider = createAcpxProvider({
  agent: 'claude',
  cwd: process.cwd(),
})

const { text } = await generateText({
  model: provider.languageModel(),
  prompt: 'Summarize this repo in 3 bullets.',
})

console.log(text)
```

That's the full setup. `acpx` will `npx`-fetch the Claude Code ACP
adapter on first run; subsequent runs are warm.

### Streaming

```ts
import { createAcpxProvider } from 'acpx-ai-provider'
import { streamText } from 'ai'

const provider = createAcpxProvider({ agent: 'claude' })

const { textStream } = streamText({
  model: provider.languageModel(),
  prompt: 'Write a haiku about TypeScript.',
})

for await (const chunk of textStream) process.stdout.write(chunk)
```

## Configuration

```ts
createAcpxProvider({
  agent: 'claude',                 // any acpx built-in id, or a custom override
  cwd: '/path/to/repo',            // working dir for the agent (default: process.cwd())
  sessionKey: 'my-session',        // logical name (default: `${agent}::${cwd}`)
  sessionMode: 'persistent',       // 'persistent' (default) reuses across turns; 'oneshot' disposes
  permissionMode: 'approve-reads', // 'approve-all' | 'approve-reads' | 'deny-all'
  nonInteractivePermissions: 'deny',
  resumeSessionId: 'sid-xyz',      // resume a prior session
  turnTimeoutMs: 60_000,
  stateDir: '~/.acpx',             // session store location
  mcpServers: [/* … */],
  agentRegistryOverrides: {
    'my-agent': 'node ./bin/my-agent.js --acp',
  },
  // advanced: inject a pre-built runtime (testing, multi-provider sharing)
  runtime: customRuntime,
})
```

Built-in agents `acpx` ships:
`pi`, `openclaw`, `codex`, `claude`, `gemini`, `cursor`, `copilot`,
`droid`, `iflow`, `kilocode`, `kimi`, `kiro`, `opencode`, `qoder`, `qwen`,
`trae`. Behavior varies — see [Known limitations](#known-limitations).

## Custom agents

The built-in registry is convenience for popular agents. **Any
binary or script that speaks ACP over stdio works** — register it
through `agentRegistryOverrides`:

```ts
const provider = createAcpxProvider({
  agent: 'my-acp-server',
  agentRegistryOverrides: {
    'my-acp-server': './bin/my-acp-server --stdio',
    // anything that produces an ACP-over-stdio process is fine:
    //   'my-acp-server': 'node ./script.js --acp',
    //   'my-acp-server': 'npx @my-org/acp-adapter@1.2.3',
  },
})
```

For your agent to drop in cleanly it must:

- Speak the [ACP](https://agentclientprotocol.com) JSON-RPC handshake
  over **stdio** (no HTTP / SSE / WebSocket transports — the runtime
  is stdio-only)
- Handle `initialize`, `session/new` (and optionally `session/load`
  for persistent sessions), and `session/prompt`
- Emit `session/update` events with the standard tool-call status
  transitions (`pending` → `in_progress` → `completed` / `failed`)
- Return one of the documented stop reasons — `end_turn` /
  `stop_sequence` / `max_tokens` / `tool_calls` / `tool_use`. Anything
  else maps to AI SDK `finishReason: 'unknown'`.

What you **don't** get for custom agents:

- No `npx` auto-download. The command runs as-is — make sure the
  binary is on PATH or use an absolute path
- No smoke-test matrix coverage. Real-world stability is on you
- Credential management is yours — the agent reads its own env vars
  or config files

If your agent is a publicly-distributed ACP adapter, the better path
is a PR to [`acpx`](https://github.com/openclaw/acpx) adding it to
the built-in registry — that gets it `npx`-auto-download for everyone
else too.

## Authentication

`acpx/runtime` reads credentials from the environment or
`~/.acpx/config.json`. There is **no programmatic credential injection**
in this provider, and **no lazy-retry on auth failure** — if a credential
is missing, the first call surfaces an `AcpxAuthRequiredError`.

```bash
# Set whichever env var the agent needs:
export ACPX_AUTH_OPENAI_API_KEY=sk-…
export ACPX_AUTH_ANTHROPIC_API_KEY=sk-ant-…
```

For agents that require an external CLI auth (e.g. GitHub Copilot),
authenticate the CLI before constructing the provider:

```bash
gh auth login
```

## Persistent sessions

By default the provider keeps a session alive across calls so the agent
preserves context. Each `languageModel()` instance for the same
`sessionKey` shares the underlying ACP session.

```ts
const provider = createAcpxProvider({ agent: 'claude' })
const model = provider.languageModel()

await generateText({ model, prompt: 'Hi, my name is Alice.' })
await generateText({ model, prompt: "What's my name?" }) // remembers

await provider.close() // tear down when done
```

Pre-warm a session without sending a prompt:

```ts
await provider.prepare()
```

Run a single isolated turn:

```ts
createAcpxProvider({ agent: 'claude', sessionMode: 'oneshot' })
```

### System prompts and per-session agent options

Pass `sessionOptions` to set the agent's `systemPrompt` (and optionally
`model`, `allowedTools`, `maxTurns`) on a fresh session. The values are
forwarded to ACP's `session/new` `_meta` and applied before the first
turn.

```ts
const provider = createAcpxProvider({
  agent: 'claude',
  sessionOptions: {
    systemPrompt: 'You are an expert Rust reviewer. Be terse.',
    // model: 'claude-opus-4-7',
    // allowedTools: ['read', 'edit'],
    // maxTurns: 5,
  },
})
```

Use `{ append: '…' }` to append to the agent's default prompt instead of
replacing it:

```ts
sessionOptions: {
  systemPrompt: { append: 'When you finish, also propose tests.' },
}
```

System prompts are fixed at `session/new` time. To switch prompts for the
same workspace, use a distinct `sessionKey`. Changing `sessionOptions`
and re-using the same key is a no-op for reused records by design — and
note that `provider.close()` does not clear the persistent record either,
so it won't force a fresh `session/new` on its own.

Not every agent honors every option — Codex / Gemini ignore Claude-specific
fields like `model`, and so on. Unrecognized options are dropped silently
at the ACP layer.

## Reasoning and plan steps

Most ACP agents stream their chain-of-thought as the turn progresses.
The provider surfaces these as AI SDK reasoning parts, so consumers
can render them with the same code they already use for any other
reasoning-capable model:

```ts
import { streamText } from 'ai'

const result = streamText({
  model: provider.languageModel(),
  prompt: 'Refactor user.ts to use Result<T, E>',
})

for await (const part of result.fullStream) {
  if (part.type === 'reasoning-delta') {
    ui.appendThinking(part.delta) // streaming "💭…" bubble
  } else if (part.type === 'text-delta') {
    ui.appendAnswer(part.delta)
  }
}
```

When the agent emits a **plan** ("I will: 1. read file 2. fix bug
3. test"), the provider surfaces it through the same channel as a
self-contained reasoning block prefixed with `[Plan]`:

```
reasoning-start (id-1)
reasoning-delta (id-1, "[Plan] 1. read file 2. fix bug 3. test")
reasoning-end   (id-1)
```

Plan blocks have their own block ids and don't disturb any
in-progress thought block — the agent can keep streaming reasoning
into one id while plan announcements come and go on others.

Agents that don't emit reasoning (e.g. Gemini CLI in some
configurations) simply produce no `reasoning-*` parts; consumer
code with a `reasoning-delta` branch never fires, no special-case
needed.

### Controlling reasoning effort

Most thinking-capable agents accept a `reasoning_effort` config
option that trades latency for depth — higher effort means more
chain-of-thought tokens before the agent answers. Set it before
the next turn:

```ts
const provider = createAcpxProvider({ agent: 'claude' })
await provider.setConfigOption('reasoning_effort', 'high')
```

What's confirmed today:

| Agent | Config key | Values | Default |
|---|---|---|---|
| `claude` | `reasoning_effort` | `low` / `medium` / `high` / `xhigh` | `medium` |
| `codex` | `reasoning_effort` (CLI alias `thought_level`) | `low` / `medium` / `high` / `xhigh` | `medium` |
| `gemini`, `copilot`, `cursor`, others | not yet documented | — | — |

The CLI alias `thought_level` is **Codex-only** — `acpx codex set
thought_level high` translates to `reasoning_effort = high` before
dispatch. Other agents take `reasoning_effort` verbatim.

### Discovering an agent's config keys

For agents not in the table — or new ones added to `acpx` — the
config-option vocabulary is per-agent. Three ways to discover:

1. **Runtime capabilities.** Ask the agent which keys it advertises:

   ```ts
   const handle = await provider.prepare()
   const caps = await provider.runtime.getCapabilities?.({ handle })
   console.log(caps?.configOptionKeys)
   ```

2. **`acpx` CLI.** Install acpx globally and inspect the agent's
   command tree (`acpx <agent>`).

3. **The adapter's source.** Every published ACP adapter has a
   `session/set_config_option` handler that lists the keys it
   accepts.

### Caveats

- **Effort changes apply to the next turn**, not the in-progress
  one. Calling `setConfigOption` mid-stream takes effect on the
  next `streamText` / `generateText` call.
- **Switching models doesn't reset effort.** A subsequent
  `setConfigOption('model', …)` keeps the previously-set
  reasoning effort; reset explicitly if you want defaults.
- **For agents not in the table, use the discovery section
  above.** Unrecognized config keys surface as an error on the
  next turn, so trial-and-error against an agent's actual
  capability list is safe.

## Tools — via MCP servers

Tools are defined through MCP (Model Context Protocol) servers passed
into `mcpServers`. The agent discovers and calls them; results flow back
through the provider's stream as `tool-call` / `tool-result` parts.

```ts
const provider = createAcpxProvider({
  agent: 'claude',
  mcpServers: [
    {
      type: 'stdio',
      name: 'filesystem',
      command: 'npx',
      args: ['-y', '@modelcontextprotocol/server-filesystem', '/tmp'],
      env: { LOG_LEVEL: 'info' },          // stdio servers — env as a record
    },
    {
      type: 'http',
      name: 'remote',
      url: 'https://example.com/mcp',
      headers: { Authorization: 'Bearer …' }, // http/sse servers — headers as a record
    },
  ],
})
```

`env` and `headers` are accepted as plain `Record<string, string>` for
ergonomics; the provider converts them to the ACP wire format
(`Array<{ name, value }>`) before handing the config to the runtime.

> **Note**: host-side AI SDK tools (the `acpTools()` /
> TCP-callback story from `acp-ai-provider`) are **not** supported in
> v0.1. See [Known limitations](#known-limitations).

## Per-call permissions

By default, every permission request the agent issues (write a file,
run a shell command, delete, etc.) is resolved by the up-front
`permissionMode` setting. To intercept individual requests with your
own UI, pass an `onPermissionRequest` callback:

```ts
const provider = createAcpxProvider({
  agent: 'codex',
  cwd: '/path/to/repo',
  permissionMode: 'approve-reads', // fallback for unhandled cases
  onPermissionRequest: async (req, { signal }) => {
    // The agent is paused mid-turn waiting for your decision.
    // Honor `signal` so a turn cancel doesn't leave it hanging.
    const decision = await myUi.prompt({
      title: req.raw.toolCall.title,
      kind: req.inferredKind, // 'edit' | 'shell' | 'delete' | …
      args: req.raw.toolCall.input,
    })
    return decision
    // Returning `undefined` falls through to the mode-based resolver.
  },
})
```

The callback receives:

| Field | Meaning |
|---|---|
| `req.sessionId` | ACP session id (handy for multi-session hosts) |
| `req.raw` | Full original `RequestPermissionRequest` from the ACP SDK |
| `req.inferredKind` | One of `'read' \| 'search' \| 'edit' \| 'delete' \| 'move' \| 'execute' \| 'fetch' \| 'think' \| 'other'` — best-effort classification from the tool's title |
| `ctx.signal` | Aborts when the turn is cancelled or the session closes |

Return one of:

- `{ outcome: 'allow_once' }` — approve this single call
- `{ outcome: 'allow_always' }` — approve this kind for the rest of the turn
- `{ outcome: 'reject_once' }` — deny this call; agent continues with the rest of its task
- `{ outcome: 'reject_always' }` — deny and remember for the rest of the turn
- `{ outcome: 'cancel' }` — agent treats the call as cancelled (often ends the turn)
- `undefined` — fall through to the mode-based resolver

**Important caveats:**

- The callback is invoked **only** when the provider builds its own
  runtime. If you pass a pre-built `runtime` via the `runtime`
  setting, set `onPermissionRequest` on that runtime instead.
- Throwing inside the callback falls through to mode-based logic and
  is logged by the runtime. Don't let UI errors take the whole turn
  down.
- The agent is **paused** until your promise resolves. There's no
  timeout enforced by the provider — wire your own (or rely on the
  agent's internal timeout, typically 5–10 minutes).

## Listing models

Some agents (Claude Code, Codex) advertise the models they can drive
when the session opens. Use `getModels()` to read both the available
list and the currently selected id:

```ts
const models = await provider.getModels()
if (models) {
  console.log(models.availableModelIds) // ['claude-haiku-4-5', …]
  console.log(models.currentModelId) // 'claude-opus-4-7'
}
```

Returns `undefined` when:

- The agent didn't advertise any models (e.g. Gemini CLI, custom
  adapters that omit `NewSessionResponse.models`).
- The underlying runtime doesn't implement `getStatus`.

`getModels()` lazily spawns the ACP session if it isn't already
open — same as `prepare()`. For multi-session providers, pass the
same `{ sessionKey, agent }` you'd pass to `languageModel()`:

```ts
const models = await provider.getModels({ sessionKey: 'codex::/repo' })
```

To **change** the model, use `setConfigOption('model', id)` from
[Lifecycle controls](#lifecycle-controls).

## Structured output (JSON)

`generateObject` / `streamObject` work via JSON mode. The provider
prepends a structured-output instruction to the prompt and strips
markdown fences (` ```json … ``` `) from the output stream so AI SDK's
parser sees clean JSON.

```ts
import { generateObject } from 'ai'
import { z } from 'zod'

const { object } = await generateObject({
  model: provider.languageModel(),
  schema: z.object({
    name: z.string(),
    ingredients: z.array(z.string()),
  }),
  prompt: 'Give me a recipe for chocolate chip cookies.',
})
```

Works with `streamObject` too.

## Lifecycle controls

```ts
await provider.cancel('user pressed stop')   // cancel the in-flight turn
await provider.setMode('plan')               // switch session mode (if agent supports it)
await provider.setConfigOption('model', 'opus') // adjust an agent config option
const report = await provider.doctor()       // diagnostic info from the runtime
await provider.close('done')                 // dispose all sessions
```

`setMode`, `setConfigOption`, and `doctor` no-op when the underlying
agent doesn't implement them. Inspect
`provider.runtime.getCapabilities()` to see what's supported.

## Known limitations

This is alpha software. Most rough edges flow through from
`acpx/runtime`, which is itself pre-1.0.

### Inherited from `acpx/runtime`

- **Tool input and output share one text field.** `tool-call.input`
  and `tool-result.result` are the same string. The runtime collapses
  both into one `text` field; the underlying ACP protocol has them
  separately, but the runtime's normalizer drops the distinction.
- **Tool input is a raw string, not parsed JSON.** `JSON.parse` it
  yourself when the agent emits valid JSON; expect failures otherwise.
- **No input/output token split.** Only `cachedInputTokens` flows
  through to AI SDK. `inputTokens`, `outputTokens`, and `totalTokens`
  are `undefined`. Per-token cost calculation won't work.
- **No streaming usage updates.** Only the most recent
  `usage_update` from the runtime survives onto the `finish` part.
- **Permission policy is mode-based by default.** When you don't
  provide an `onPermissionRequest` callback, requests fall through to
  `permissionMode` + `nonInteractivePermissions` — same as before.
  Hosts wanting per-call gating should set the callback (see
  [Per-call permissions](#per-call-permissions)).
- **Auth is env-var / config-file driven, no lazy retry.** Missing
  credentials throw at first use.
- **`npx` cold start on first agent use.** Built-in agents
  auto-download via `npx`. First call after a clean install can take
  10+ seconds.
- **Sessions persist on the filesystem at `~/.acpx/sessions/`.** Not
  multi-process-safe by default. Override `stateDir` if needed.
- **Mid-turn `AbortSignal` honoring varies by agent.** We forward the
  signal to `runtime.startTurn`, but how quickly the agent stops
  varies. `provider.cancel()` is the strongest signal.
- **Optional control methods may no-op.** `setMode`,
  `setConfigOption`, `doctor`, `getStatus` aren't implemented by every
  agent.

### AI SDK integration

- **`LanguageModelV2` compatibility-mode warning.** AI SDK v6 prints a
  warning on first use ("specificationVersion is used in a
  compatibility mode"). Harmless. Will go away when we move to V3.
- **No host-side AI SDK tools.** v0.1 only supports tools the agent
  learns about via MCP servers. AI SDK `tool({ execute })` callbacks
  passed to `streamText` won't be invoked from this provider — the
  agent doesn't know about them.
- **`tool-call.providerExecuted` is always `true`.** Every tool call
  is marked as already-executed by the agent.
- **Multi-step / `stopWhen`.** AI SDK's loop works at the SDK level,
  but each step is a fresh `runtime.startTurn`. Use the default
  `sessionMode: 'persistent'` so the agent keeps its own context
  across steps.
- **`generateObject` / `streamObject`** work via JSON mode. Agents
  that aren't JSON-strict may emit malformed JSON; the fence-stripping
  transform handles markdown wrappers, not bad JSON.
- **`request.body` and `response.headers` are synthetic.**
  `request.body` is `{ agent, sessionKey }`; `response.headers` is
  always `{}`. We have no HTTP layer.

### Per-agent quirks

Behavior varies by built-in agent. Recommended starting matrix:

| Agent | Notes |
|---|---|
| `claude` | Best-tested path. Clean text + tools. |
| `codex` | JSON output benefits most from the fence cleanup. |
| `pi` | Cheapest for smoke tests. |
| `gemini` | Requires `--experimental-acp` (registry already passes it). Capability surface still evolving. |
| `copilot` | Requires authenticated GitHub Copilot CLI. Run `gh auth login` first. |

### Out of scope for v0.1

These are deliberate non-goals, not bugs:

- Host-side AI SDK tools.
- Mid-stream model switching from inside a single `streamText` call.
- Provider-defined dynamic tool routing.
- Live token-cost calculation.
- Per-`languageModel()` agent registry.

## Errors

```ts
import {
  AcpxError,
  AcpxAgentNotFoundError,
  AcpxAuthRequiredError,
  AcpxTurnTimeoutError,
} from 'acpx-ai-provider'
```

Catch `AcpxError` for the broad case; the three subclasses cover the
common diagnosable causes. Anything else falls through to the base
class with the runtime's `code` preserved.

## Repository

Source, issues, and roadmap: <https://github.com/DaniAkash/acpx>.

## License

MIT © Dani Akash
