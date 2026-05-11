# Custom ACP Agent Configuration via acpx

**Date**: 2026-05-11
**Status**: Verified Draft (7 sub-agent verification round)
**Depends on**: acpx@0.6.1, Agent Client Protocol spec

---

## User Requirements (verbatim)

> 1. https://github.com/openclaw/acpx — acpx can support lots of others agents.
>
> a. Make it to be able to config other agents as well (using their default bin)
> b. acpx can config custom agent as well like this:
> ```json
> {
>   "defaultAgent": "codex",
>   "defaultPermissions": "approve-all",
>   "nonInteractivePermissions": "deny",
>   "authPolicy": "skip",
>   "ttl": 300,
>   "timeout": null,
>   "format": "text",
>   "agents": {
>     "my-custom": { "command": "./bin/my-acp-server", "args": ["acp"] }
>   },
>   "auth": {
>     "my_auth_method_id": "credential-value"
> }
> }
> ```
> Implement a section to be able to config that as well, basically it will be any arbitrary cmd that will launch the underlying acp. We will start the conversation in the acpx to probes if it is in correct format, if not then make the warning, do not block the rest. Make the TEST button per configuration as well.
>
> c. acpx is having it unify directory for config, make our configuration can be doing these 2:
> c1. load and run the configured agents of acpx
> c2. can custom acpx config directory for that loading
> c3. import these agents from acpx configuration into browserOS
>
> Each eventually will just the configuration layer and the underlying is the acpx itself.

---

## Architecture

```
┌─────────────────────────────────────────────────┐
│ BrowserOS UI (agents/NewAgentDialog.tsx)         │
│                                                  │
│  Adapter dropdown: Claude/Codex/OpenClaw/Hermes/ │
│  Custom ACP Agent                                │
│                                                  │
│  Custom fields: Command + Args                   │
│  TEST button: POST /agents/probe-custom          │
│  Import button: POST /agents/import-acpx         │
└───────────────────────┬─────────────────────────┘
                        │
                        ▼
┌─────────────────────────────────────────────────┐
│ Existing POST /agents/ route (extended)          │
│  + optional customCommand/customArgs fields      │
└───────────────────────┬─────────────────────────┘
                        │
                        ▼
┌─────────────────────────────────────────────────┐
│ AgentHarnessService.createAgent()                │
│  + custom branch: no OpenClaw/Hermes provisioning│
└───────────────────────┬─────────────────────────┘
                        │
                        ▼
┌─────────────────────────────────────────────────┐
│ AcpxRuntime                                      │
│  ensureSession(agent: 'custom:<agentId>')        │
│  resolve() → wrapCommandWithEnv(customCommand)   │
└───────────────────────┬─────────────────────────┘
                        │ stdio JSON-RPC (ACP)
                        ▼
                  Any ACP binary
```

---

## Data Model

### AgentAdapter type extension

```ts
// agent-types.ts
export type AgentAdapter = 'claude' | 'codex' | 'openclaw' | 'hermes' | 'custom'

export interface AgentDefinition {
  id: string
  name: string
  adapter: AgentAdapter
  modelId?: string
  reasoningEffort?: string
  permissionMode: AgentPermissionMode
  sessionKey: string
  createdAt: number
  updatedAt: number
  pinned?: boolean
  // NEW: stored in adapterConfigJson, not new columns
  customCommand?: string    // required when adapter='custom'
  customArgs?: string[]     // optional
}
```

**Note**: No `customLabel` field. `name` serves as display name (YAGNI per v-data-model).

### AgentAdapterDescriptor catalog extension

```ts
// agent-catalog.ts
{
  id: 'custom',
  name: 'Custom ACP Agent',
  defaultModelId: 'default',
  defaultReasoningEffort: 'medium',
  modelControl: 'best-effort',
  models: [],  // no per-session model picker
  reasoningEfforts: [
    { id: 'low', label: 'Low' },
    { id: 'medium', label: 'Medium', recommended: true },
    { id: 'high', label: 'High' },
  ],
}
```

### DB schema

**Schema change**: Extend `adapter` enum in `lib/db/schema/agents.ts`:
```ts
adapter: text('adapter', { enum: ['claude', 'codex', 'openclaw', 'hermes', 'custom'] }).notNull()
```

**No new columns.** Custom fields stored in existing `adapterConfigJson`:
```json
{ "customCommand": "gemini", "customArgs": ["--acp"] }
```

**MUST extend** `serializeAdapterConfig()` and `toAgentDefinition()` in `db-agent-store.ts`:
- `serializeAdapterConfig()`: add `customCommand`/`customArgs` to serialized JSON
- `toAgentDefinition()`: parse `adapterConfigJson` and hydrate `customCommand`/`customArgs` back

Without this, custom fields are silently lost on write and never read back (verified by v-data-model + v-gotchas).

### Config store

**Only one new key** (ACPX.AUTO_IMPORT deferred to v2 — YAGNI per v-data-model):

```ts
// config-schema.ts — new string config key
'ACPX.CONFIG_DIR': {
  default: '~/.acpx',
  type: 'string',
  min: 1,
  description: 'Path to acpx config directory for agent import',
}
```

**Implementation note**: Current `config-schema.ts` only supports numeric types (`ConfigValueSchema = z.number()`). Need to add a parallel string config type (`StringConfigKeyMeta` + `STRING_CONFIG_KEYS`) or refactor `ConfigKeyMeta` to a discriminated union (verified by v-data-model).

---

## Component Details

### 1. acpx-config-sync.ts (NEW — single file, no separate acpx-probe.ts)

Location: `apps/server/src/api/services/agents/acpx-config-sync.ts`

```
readAcpxConfig(acpxDir?: string) → AcpxConfig | null
  - Default dir: configStore.get('ACPX.CONFIG_DIR') resolved (~ → $HOME)
  - Reads config.json from dir directly (no acpx CLI dependency)
  - Returns { agents: { [name]: { command, args } } } or null

importAgentsFromAcpx(config, existingAgents) → ImportResult[]
  - For each entry in config.agents:
    - Skip if agent with same name already exists (idempotent)
    - Create AgentDefinition { adapter: 'custom', customCommand, customArgs, name }
    - Agents named 'claude'/'codex'/'openclaw'/'hermes': import as custom + show warning
  - Return results list

probeCustomAgent(command: string, args: string[]) → ProbeResult
  - Spawn child process with shell: true
  - Send ACP initialize JSON-RPC over stdin
  - Read stdout for response (15s timeout, SIGKILL fallback)
  - Return { healthy: boolean, error?: string }
  - On timeout + command contains 'npx': append hint about first-run download
  - agentInfo { name, version }: parsed from initialize response if present (diagnostic only, no UI dependency)
  - Never throws — returns { healthy: false, error } on all failures
```

**Dropped** (YAGNI per v-acpx-sync):
- ~~`exportAgentToAcpx()`~~ — BrowserOS is source of truth, no dual-write
- ~~`getAcpxConfig()`~~ — import endpoint returns discovery data inline

### 2. AcpxRuntime changes

**Do NOT change `resolve()` signature** — it's an acpx interface contract (verified by v-acpx-runtime).

Instead:

1. Pass `Map<string, AgentDefinition>` into `createBrowserosAgentRegistry()`
2. Use `custom:<agentId>` as the agent name in `ensureSession()`
3. Add a `custom:*` branch in BrowserOS's resolve wrapper:

```ts
// In createBrowserosAgentRegistry().resolve():
if (agentName.startsWith('custom:')) {
  const agentId = agentName.slice('custom:'.length)
  const def = input.customAgents.get(agentId)
  if (!def?.customCommand) return agentName  // fallback
  return wrapCommandWithEnv(
    [def.customCommand, ...(def.customArgs ?? [])].join(' '),
    input.commandEnv
  )
}
```

4. In `createAcpxEventStream()`, pass unique agent name:
```ts
agent: input.agent.adapter === 'custom'
  ? `custom:${input.agent.id}`
  : input.agent.adapter
```

5. Thread `AgentDefinition` through: `send()` → `getRuntime()` → `createBrowserosAgentRegistry()`

6. **Runtime cache key**: include custom command string to prevent cross-agent collisions

### 3. Agent Harness Service changes

**No new methods** — keep it minimal:

- `createAgent()`: existing method, just add `adapter === 'custom'` branch (skip OpenClaw/Hermes provisioning)
- Probe and import logic lives in `acpx-config-sync.ts`, called directly from route handlers

### 4. API routes (minimal — 2 new routes only)

```
POST  /agents/probe-custom              ← TEST button (pre-creation)
  Body: { command: string, args?: string[] }
  → ProbeResult { healthy, error? }
  → Always 200. Warning only.

POST  /agents/import-acpx               ← Import button
  Body: { acpxDir?: string }
  → { results: ImportResult[], discovered: number }
```

**Dropped** (YAGNI per v-harness-routes):
- ~~`POST /agents/custom`~~ — extend existing `POST /agents/` with optional customCommand/customArgs
- ~~`GET /agents/acpx-config`~~ — discovery data returned inline from import
- ~~`POST /agents/:id/probe`~~ — probe-custom takes command+args directly, no agentId needed

### 5. UI changes — extend existing dialog

**Extend `agents/NewAgentDialog.tsx`**, not new files (verified by v-ui).

When adapter dropdown selects "Custom ACP Agent":
- **Command** input (required)
- **Args** input (optional)
- **TEST** button → `POST /agents/probe-custom { command, args }` → shows ✅ or ⚠️
- **Import from acpx** button → `POST /agents/import-acpx {}` → shows results summary ("3 imported, 1 skipped")

No checkbox selection — import-all with summary is simpler (YAGNI per v-ui).

---

## Mandatory Code Changes (from verifier findings)

These are **breaking if not done** — not optional:

| # | File | Change | Verified by |
|---|------|--------|-------------|
| M1 | `agent-catalog.ts` | `isAgentAdapter()`: add `value === 'custom'` | v-data-model |
| M2 | `acpx-agent-adapter.ts` | `ADAPTERS` record: add `custom: { prepare: prepareCustomContext }` | v-data-model, v-gotchas N1 |
| M3 | `acpx-agent-adapter.ts` | New `prepareCustomContext()` function (reuse common flow) | v-acpx-runtime |
| M4 | `db-agent-store.ts` | `serializeAdapterConfig()`: include customCommand/customArgs | v-data-model, v-gotchas N4 |
| M5 | `db-agent-store.ts` | `toAgentDefinition()`: parse adapterConfigJson → custom fields | v-data-model, v-gotchas N4 |
| M6 | `lib/db/schema/agents.ts` | Extend adapter enum with `'custom'` | v-data-model H1 |
| M7 | `agent-store.ts` | `CreateAgentInput`: add `customCommand?`, `customArgs?` | v-data-model M2 |
| M8 | `routes/agents.ts` | `parseCreateAgentBody()`: extract customCommand/customArgs | v-data-model M2 |
| M9 | `routes/agents.ts` | Model validation: skip catalog check for `adapter === 'custom'` (like openclaw/hermes) | v-data-model M1 |
| M10 | `acpx-runtime.ts` | Registry: accept `Map<string, AgentDefinition>`, add `custom:*` resolve branch | v-acpx-runtime |
| M11 | `acpx-runtime.ts` | `createAcpxEventStream()`: pass `custom:<agentId>` when adapter=custom | v-acpx-runtime |
| M12 | `adapter-health.ts` | Add `adapter === 'custom'` bypass (return healthy like openclaw fallback) | v-gotchas N2 |
| M13 | `config-schema.ts` | Add string config type support + `ACPX.CONFIG_DIR` key | v-data-model H3 |

---

## Gotchas & Edge Cases

### G0: Drizzle enum constraint (CRITICAL)
Must add `'custom'` to adapter enum. Requires schema change + migration.

### G1: Command with spaces
Use `child_process.spawn(command, args, { shell: true })`.

### G2: acpx not installed
Read config JSON directly, no acpx CLI dependency. Probe spawns binary directly.

### G3: Custom agent name collision
`adapter='custom'` never shadows built-ins. Harness routes by `adapter` field. Import of agent named 'claude' from acpx config works but shows warning.

### G3b: isAgentAdapter() — see M1 above

### G3c: AcpxRuntime resolve() — see M10-M11 above. NOT changing resolve() signature.

### G3d: OpenClaw dual-tracking — `adapter !== 'openclaw'` guard correctly skips custom agents

### G4: acpx config missing → returns null, UI shows "no config found"

### G5: Probe timeout → 15s + SIGKILL. Never blocks.

### G6: Crash reconnect → AcpxRuntime handles for all agents equally.

### ~~G7: Auth~~ — DROPPED. Auth stays in acpx config. BrowserOS does not read it. (YAGNI per v-gotchas)

### G8: Import idempotency — skip same-name, update command if changed.

### G9: AgentRuntimeRegistry single 'custom' slot (NEW from v-gotchas N3)
Registry maps `adapterId → AgentRuntime`. All custom agents share `adapter='custom'`. Health/probe path must bypass registry and spawn directly. The per-agent command resolution happens inside `createBrowserosAgentRegistry().resolve()`.

### G10: Runtime cache key collision (NEW from v-acpx-runtime)
Different custom agents with same cwd/env could share cached runtime. Fix: include custom command string in cache key.

### G11: acpx agent named 'claude' in config (NEW from v-acpx-sync)
Import as `adapter='custom'` with warning. Do NOT skip — user configured it intentionally.

---

## Files to Create/Modify

### New files
| File | Purpose |
|------|---------|
| `apps/server/src/api/services/agents/acpx-config-sync.ts` | read/import/probe acpx config |
| `apps/server/src/lib/agents/runtime/custom-host-process-runtime.ts` | prepareCustomContext |
| `apps/server/tests/api/services/agents/acpx-config-sync.test.ts` | Unit tests |

### Modified files
| File | Change |
|------|--------|
| `lib/agents/agent-types.ts` | Add `'custom'` to AgentAdapter + customCommand/customArgs |
| `lib/agents/agent-catalog.ts` | Add custom descriptor + update `isAgentAdapter()` |
| `lib/agents/acpx-agent-adapter.ts` | Add `'custom'` entry to ADAPTERS record |
| `lib/agents/acpx-runtime.ts` | Custom agent registry resolution + ensureSession agent name |
| `lib/db/schema/agents.ts` | Extend adapter enum |
| `lib/agents/db-agent-store.ts` | Serialize/deserialize custom fields in adapterConfigJson |
| `lib/agents/agent-store.ts` | Add custom fields to CreateAgentInput |
| `lib/agents/adapter-health.ts` | Add 'custom' bypass |
| `api/routes/agents.ts` | Extend create route + probe-custom + import-acpx routes |
| `packages/shared/src/constants/config-schema.ts` | String config type + ACPX.CONFIG_DIR |
| `agents/NewAgentDialog.tsx` | Add Custom ACP option + command/args fields + TEST + import |

---

## Testing Strategy

### Unit tests
- `acpx-config-sync.test.ts`: read config (valid/invalid/missing), import (new/existing/collision, name-clash with built-in), probe (success/timeout/command-not-found/npx-hint)
- `agent-catalog.test.ts`: `isAgentAdapter('custom')` returns true, custom adapter validation

### Integration tests
- Create custom agent via `POST /agents/` → verify customCommand in DB → probe
- Import from acpx config → verify agents created → re-import idempotent

### TDD order
1. RED: acpx-config-sync tests (read, import, probe)
2. GREEN: acpx-config-sync implementation
3. RED: agent-catalog + agent-types tests (isAgentAdapter, custom validation)
4. GREEN: type + catalog changes
5. RED: route tests (create custom, probe-custom, import-acpx)
6. GREEN: routes + harness + store plumbing
7. RED: AcpxRuntime custom resolution test
8. GREEN: runtime registry changes
9. UI: extend NewAgentDialog (no TDD)
