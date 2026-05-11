# Custom ACP Agent Configuration via acpx

**Date**: 2026-05-11
**Status**: Draft
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
>   }
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

### Approach: Thin Config Layer + acpx Config Pass-Through (Hybrid)

BrowserOS reads acpx config for import (c1/c2/c3), but also has its own custom agent form (a/b) that writes back to acpx config. Single source of truth with full UI control.

```
┌─────────────────────────────────────────────────┐
│ BrowserOS UI                                     │
│                                                  │
│  ┌──────────────────┐  ┌──────────────────────┐ │
│  │ Create Agent      │  │ Import from acpx     │ │
│  │ - Adapter dropdown│  │ - Scan ~/.acpx/      │ │
│  │ - Custom command  │  │ - Pick agents        │ │
│  │ - TEST button     │  │ - Import selected    │ │
│  └────────┬─────────┘  └──────────┬───────────┘ │
│           │                       │              │
│           ▼                       ▼              │
│  ┌──────────────────────────────────────────┐   │
│  │ AgentHarnessService                       │   │
│  │  - createAgent(adapter, customCommand..)  │   │
│  │  - probeAgent(agentId) → ProbeResult      │   │
│  │  - importFromAcpx(acpxDir)                │   │
│  └──────────────────┬───────────────────────┘   │
│                     │                            │
│  ┌──────────────────▼───────────────────────┐   │
│  │ AcpxRuntime                               │   │
│  │  - resolve('custom') → spawn command      │   │
│  │  - Same ACP handshake as built-in agents  │   │
│  └──────────────────────────────────────────┘   │
└─────────────────────────────────────────────────┘
         │
         │ stdio JSON-RPC (ACP)
         ▼
┌─────────────────────────────────────────────────┐
│ acpx runtime (npm:acpx@0.6.1)                    │
│  - createAgentRegistry()                         │
│  - Agent spawn + session management              │
│  - Custom agents resolve via command string      │
└─────────────────────────────────────────────────┘
         │
         │ spawns
         ▼
┌─────────────────────────────────────────────────┐
│ Any ACP-capable binary                           │
│  gemini, cursor, copilot, pi, opencode, kiro,    │
│  kimi, trae, droid, kilocode, qwen, qoder,       │
│  iflow, or user's custom script                  │
└─────────────────────────────────────────────────┘
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
  // NEW fields (required when adapter='custom')
  customCommand?: string    // e.g. "gemini", "./bin/my-acp", "npx -y opencode-ai acp"
  customArgs?: string[]     // e.g. ["acp", "--profile", "ci"]
  customLabel?: string      // display name override for UI
}
```

### AgentAdapterDescriptor catalog extension

```ts
// agent-catalog.ts — add custom descriptor
{
  id: 'custom',
  name: 'Custom ACP Agent',
  defaultModelId: 'default',
  defaultReasoningEffort: 'medium',
  modelControl: 'best-effort',
  models: [],  // no per-session model picker — agent binary controls this
  reasoningEfforts: [
    { id: 'low', label: 'Low' },
    { id: 'medium', label: 'Medium', recommended: true },
    { id: 'high', label: 'High' },
  ],
}
```

### DB schema

**Schema migration required** for the `adapter` enum column:

Current drizzle schema (`lib/db/schema/agents.ts`) defines:
```ts
adapter: text('adapter', { enum: ['claude', 'codex', 'openclaw', 'hermes'] }).notNull()
```

Must extend to:
```ts
adapter: text('adapter', { enum: ['claude', 'codex', 'openclaw', 'hermes', 'custom'] }).notNull()
```

**No new columns needed.** Use existing `adapterConfigJson` column to store custom agent config:
```json
{
  "customCommand": "gemini",
  "customArgs": ["--acp"],
  "customLabel": "Gemini CLI"
}
```

`adapterConfigJson` is already a nullable text column used for provider config (providerType, providerName, baseUrl, apiKey, supportsImages). Extend `serializeAdapterConfig()` and `toAgentDefinition()` to include custom fields.

### Config store new keys

```ts
// config-schema.ts
'ACPX.CONFIG_DIR': {
  default: '~/.acpx',
  type: 'string',
  min: 1,
  description: 'Path to acpx configuration directory containing config.json',
},
'ACPX.AUTO_IMPORT': {
  default: false,
  type: 'boolean',
  description: 'Automatically import new agents from acpx config on server startup',
},
```

---

## Component Details

### 1. acpx-config-sync.ts (NEW)

Location: `apps/server/src/api/services/agents/acpx-config-sync.ts`

```
readAcpxConfig(acpxDir?: string) → AcpxConfig | null
  - Default dir: configStore.get('ACPX.CONFIG_DIR') resolved (~ → $HOME)
  - Reads config.json from dir
  - Returns parsed { agents, defaultAgent, auth, ... } or null if missing/invalid

importAgentsFromAcpx(config: AcpxConfig, existingAgents: AgentDefinition[])
  → ImportResult[]
  - For each entry in config.agents:
    - Skip if agent with same name already exists (idempotent)
    - Create AgentDefinition { adapter: 'custom', customCommand, customArgs, name }
    - Return { imported: AgentDefinition, skipped: boolean, reason?: string }

exportAgentToAcpx(agent: AgentDefinition, acpxDir: string) → void
  - Read existing config.json
  - Add/update entry in agents map
  - Write back

probeAgent(command: string, args: string[], timeout?: number) → ProbeResult
  - Spawn process: `${command} ${args.join(' ')}`
  - Send ACP initialize handshake over stdin
  - Wait for initialize response (or timeout 15s)
  - Kill process
  - Return { healthy: boolean, error?: string, agentInfo?: { name, version } }
  - On failure: warning only, never throws
```

### 2. AcpxRuntime changes

In `createBrowserosAgentRegistry().resolve()`:

```ts
if (lower === 'custom') {
  // Look up the agent's customCommand from the definition
  // This requires passing agent context into resolve()
  // ...see implementation note below
}
```

Implementation note: `resolve()` currently only receives `agentName` string. We need to extend it to also accept agent metadata. Options:
- **Option A**: Pass full `AgentDefinition` through `resolve()`
- **Option B**: Store a `Map<string, AgentDefinition>` in the registry context

**Choice: Option A** — modify `resolve()` signature to accept optional `AgentDefinition` for custom agents. This is cleaner because the command comes from the agent definition, not the registry.

### 3. Agent Harness Service changes

```ts
// New methods on AgentHarnessService

probeAgent(agentId: string): Promise<ProbeResult>
  - Look up agent definition
  - If adapter !== 'custom', return { healthy: true } (built-in agents already probed)
  - Call probeAgent(customCommand, customArgs)

importFromAcpx(acpxDir?: string): Promise<ImportResult[]>
  - Read acpx config
  - Diff against existing agents
  - Create new agents for entries not yet imported
  - Return results

getAcpxConfig(): Promise<AcpxConfig | null>
  - Read acpx config from configured dir
  - Return parsed config for UI display
```

### 4. API routes

```
POST   /agents/custom
  Body: { name, customCommand, customArgs?, customLabel? }
  → Creates AgentDefinition with adapter='custom'
  → Validates customCommand is non-empty
  → Optionally writes to acpx config

POST   /agents/:id/probe
  → ProbeResult { healthy, error?, agentInfo? }
  → 15s timeout
  → Never returns error status — always 200 with probe result

POST   /agents/import-acpx
  Body: { acpxDir?: string, agentNames?: string[] }
  → Scan acpx config, import selected (or all if agentNames omitted)
  → Returns ImportResult[]

GET    /agents/acpx-config
  → Returns current acpx config (agents map) for UI display
  → { agents: { name: { command, args } }, defaultAgent, ... }
```

### 5. UI — Create Agent dialog changes

**Adapter dropdown** adds:
```
- Claude Code
- Codex
- OpenClaw
- Hermes
─────────────
- Custom ACP Agent  ← NEW
```

When "Custom ACP Agent" selected, show:
- **Name** field (agent display name)
- **Command** field (the binary/command to spawn)
- **Args** field (optional, comma-separated or multi-input)
- **TEST button** — calls `POST /agents/:id/probe` or inline probe
- **Import from acpx** collapsible section:
  - acpx directory input (default `~/.acpx`)
  - "Scan" button — calls `GET /agents/acpx-config`
  - Checkbox list of discovered agents
  - "Import Selected" button — calls `POST /agents/import-acpx`

### 6. Probe (TEST button) flow

```
User clicks TEST
  → UI calls POST /agents/probe-custom { command, args }
  → Server spawns child process:
      1. Spawn: command + args
      2. Write ACP initialize to stdin
      3. Read stdout for JSON-RPC initialize response
      4. Timeout 15s
      5. Kill process
  → Parse response:
      ✅ { healthy: true, agentInfo: { name: "gemini", version: "2.1.0" } }
      ⚠️ { healthy: false, error: "command not found: gemini" }
      ⚠️ { healthy: false, error: "process exited with code 1: ..." }
      ⚠️ { healthy: false, error: "timeout: no ACP response within 15s" }
  → UI shows result badge (green checkmark or yellow warning)
  → WARNING ONLY — does NOT disable "Create" button
```

### 7. Startup auto-import (requirement c1)

When `ACPX.AUTO_IMPORT=true` in BrowserOS config:
1. On server startup, after agent harness initializes
2. Read acpx config from `ACPX.CONFIG_DIR`
3. Diff agents map against existing BrowserOS agents
4. Auto-create any new agents found
5. Log results
6. Does NOT remove agents that were deleted from acpx config

---

## Gotchas & Edge Cases

### G0: Drizzle enum constraint (CRITICAL)
`adapter` column uses drizzle enum `['claude', 'codex', 'openclaw', 'hermes']`. Must add `'custom'` to the enum. This requires a drizzle migration file AND an SQLite `ALTER TABLE` to update the check constraint. Files: `lib/db/schema/agents.ts` + migration.

### G1: Command with spaces
Custom commands like `"npx -y opencode-ai acp"` need proper shell parsing. Use `child_process.spawn(command, args, { shell: true })` or pre-parse with `shlex`-like splitting.

### G2: acpx not installed
`readAcpxConfig()` reads JSON directly — no dependency on acpx CLI. But `probeAgent()` spawns the binary directly (not via acpx), so acpx itself is NOT required for custom agents to work.

### G3: Custom agent name collision
If user creates a custom agent named "claude", it must NOT shadow the built-in claude adapter. Custom agents always have `adapter='custom'`, so the built-in registry is unaffected. The harness routes by `adapter` field, not by name.

### G3b: isAgentAdapter() validator
`agent-catalog.ts` has `isAgentAdapter()` that checks exact values: `value === 'claude' || value === 'codex' || value === 'openclaw' || value === 'hermes'`. Must add `|| value === 'custom'`. Used in `routes/agents.ts` line 665 for validation — will reject custom agents if not updated.

### G3c: AcpxRuntime resolve() registry
`acpx-runtime.ts` `createBrowserosAgentRegistry().resolve()` has explicit branches for `openclaw`, `hermes`, `claude`, `codex`. Unknown names fall through to `registry.resolve(agentName)` which treats them as raw commands. Custom agents with `adapter='custom'` need their own branch that reads `customCommand` from the agent definition. Current `resolve()` only receives `agentName` string — needs signature change to also accept `AgentDefinition`.

### G3d: OpenClaw dual-tracking skip
`routes/agents.ts` lines 684-685 check `record.adapter !== 'openclaw'` before OpenClaw provisioning. Custom agents correctly skip this. But the createAgent flow in `agent-harness-service.ts` may still attempt gateway provisioning — verify the guard covers all paths.

### G4: acpx config directory missing
`readAcpxConfig()` returns `null` gracefully. UI shows "No acpx configuration found" message.

### G5: Probe process hangs
15s timeout + SIGKILL fallback. Never blocks the server.

### G6: Custom agent exits mid-session
AcpxRuntime already handles crash reconnect for all agents. Custom agents get the same treatment — dead process detected, session reloaded transparently.

### G7: Auth for custom agents
acpx config `auth` map is read but NOT used by BrowserOS directly. Auth credentials stay in acpx config. BrowserOS passes through environment variables to child processes (existing behavior).

### G8: Import idempotency
Re-importing same agent name updates `customCommand`/`customArgs` if they changed, does NOT create duplicate.

---

## Files to Create/Modify

### New files
| File | Purpose |
|------|---------|
| `apps/server/src/api/services/agents/acpx-config-sync.ts` | Read/write/probe acpx config |
| `apps/server/src/api/services/agents/acpx-probe.ts` | ACP initialize handshake probe |
| `apps/agent/entrypoints/app/ai-settings/CustomAgentForm.tsx` | UI form for custom agent config |
| `apps/agent/entrypoints/app/ai-settings/AcpxImportPanel.tsx` | UI panel for acpx import |
| `tests/server/api/services/agents/acpx-config-sync.test.ts` | Unit tests |
| `tests/server/api/services/agents/acpx-probe.test.ts` | Probe tests |

### Modified files
| File | Change |
|------|--------|
| `lib/agents/agent-types.ts` | Add `'custom'` to AgentAdapter union + new fields to AgentDefinition |
| `lib/agents/agent-catalog.ts` | Add custom descriptor + update validators |
| `lib/agents/acpx-runtime.ts` | Add custom agent resolution in registry |
| `lib/db/schema/agents.ts` | Add `'custom'` to adapter enum |
| `api/services/agents/agent-harness-service.ts` | Add probe/import methods |
| `api/routes/agents.ts` | Add new routes |
| `packages/shared/src/constants/config-schema.ts` | Add ACPX.CONFIG_DIR, ACPX.AUTO_IMPORT |
| UI: Create Agent dialog | Add Custom ACP option + import panel |

---

## Testing Strategy

### Unit tests
- `acpx-config-sync.test.ts`: read config (valid/invalid/missing), import (new/existing/collision), export
- `acpx-probe.test.ts`: probe success, probe timeout, probe command-not-found, probe invalid JSON
- `agent-catalog.test.ts`: custom adapter validation, customCommand required when adapter=custom

### Integration tests
- Create custom agent via API → verify in DB → probe → send message (mock agent)
- Import from acpx config → verify agents created → re-import idempotent

### TDD approach
1. RED: Write tests for acpx-config-sync (read, import, export, probe)
2. GREEN: Implement acpx-config-sync + acpx-probe
3. RED: Write tests for API routes (create custom, probe, import)
4. GREEN: Implement routes + harness methods
5. RED: Write tests for AcpxRuntime custom resolution
6. GREEN: Implement custom agent registry resolution
7. UI: Implement form + import panel (no TDD for UI)
