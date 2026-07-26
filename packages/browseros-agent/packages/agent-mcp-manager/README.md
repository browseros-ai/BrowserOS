# agent-mcp-manager

> Programmatic add/link/unlink for **Model Context Protocol** servers
> across 23 AI coding agents. Functional API, dry-run capable, 23-client
> catalog with per-client shape declarations.

> [!WARNING]
> **v0.0.4 is a breaking release.** The `createMcpManager` class API
> has been removed and replaced with a functional surface. See the
> [Migration guide](#migration-from-v003) below for a mechanical
> translation table. If you cannot migrate immediately, pin
> `agent-mcp-manager@^0.0.3`.

> [!WARNING]
> **Experimental.** The 0.0.x line is under active development. The
> public API may change between patch versions. Pin exact versions.

`agent-mcp-manager` writes MCP server entries into the real config
files that AI coding agents read on launch. Claude Desktop's
`claude_desktop_config.json`, Cursor's `~/.cursor/mcp.json`, VS Code's
`mcp.json`, Codex's `~/.codex/config.toml`, and 19 others across
JSON / JSONC / YAML / TOML. It targets embedders (IDE plugins,
internal tools, enterprise onboarding flows, custom installers) that
need to register MCP servers programmatically across many agents.

## Install

```sh
bun add agent-mcp-manager
# or: npm i, pnpm add, yarn add
```

## Quick start

```ts
import { link, disconnect, rescan, bind } from 'agent-mcp-manager'

const workspaceDir = '~/.myapp/mcp'

// The server is caller-owned data. No pre-registration step.
const github = {
  name: 'github',
  spec: {
    transport: 'http',
    url: 'https://api.githubcopilot.com/mcp',
    headers: { Authorization: `Bearer ${process.env.GH_TOKEN}` },
  },
} as const

// One link call = one atomic operation:
//   - upserts the manifest server entry
//   - writes the entry to the agent's config file
await link(workspaceDir, { server: github, agent: 'cursor' })
await link(workspaceDir, { server: github, agent: 'claude-code' })
await link(workspaceDir, { server: github, agent: 'vscode' })

// Later, disconnect one agent without touching the others.
await disconnect(workspaceDir, {
  serverName: 'github',
  agent: 'cursor',
  removeIfLast: true, // drop the manifest entry only if no agents remain
})

// Periodically check that on-disk configs still match the manifest.
const report = await rescan(workspaceDir)
if (report.drifted.length > 0 || report.missing.length > 0) {
  // ... report / auto-heal / prompt the user
}

// For consumers who use the same workspaceDir across many calls:
const mgr = bind(workspaceDir)
await mgr.link({ server: github, agent: 'gemini' })
```

## API reference

### Verbs

| Verb | Signature | Purpose |
|---|---|---|
| `link` | `(workspaceDir, {server, agent, scope?, projectRoot?, configPath?, allowOverwrite?}) => Promise<LinkPlanSummary>` | Upsert the manifest server entry AND write the server into the agent's config file. Last-write-wins on the manifest spec. |
| `unlink` | `(workspaceDir, {serverName, agent, scope?, projectRoot?, configPath?}) => Promise<UnlinkPlanSummary>` | Remove one agent's entry from its config file and drop the manifest link. No-op when the manifest has no such link. |
| `disconnect` | `(workspaceDir, {serverName, agent, scope?, projectRoot?, removeIfLast?}) => Promise<DisconnectPlanSummary>` | Unlink one agent AND drop the manifest entry when no other agents remain linked to it. Never touches other agents' config files. **The primitive that closes [issue #63](https://github.com/DaniAkash/agent-toolkit/issues/63).** |
| `remove` | `(workspaceDir, {serverName, unlinkFirst?}) => Promise<RemovePlanSummary>` | Drop the manifest entry AND unlink every currently-linked agent's config file. |
| `list` | `(workspaceDir) => Promise<ManifestServerEntry[]>` | Every server in the manifest. |
| `listLinks` | `(workspaceDir, {serverNames?, agents?}?) => Promise<ListedLink[]>` | Every (server, agent, configPath) triple in the manifest. Filter by server name or agent. |
| `rescan` | `(workspaceDir, {agents?}?) => Promise<RescanReport>` | Diff manifest links against disk. Reports verified / drifted / missing entries. **See [rescan section](#rescan-detecting-drift-between-manifest-and-disk) below.** |
| `isInstalled` | `({agents, scope?, projectRoot?}) => Promise<Partial<Record<AgentId, boolean>>>` | Batch check whether each agent's config location is writable-safely (file exists OR parent directory exists). Predicts what `link()` would throw. **See [isInstalled section](#isinstalled-checking-agent-availability) below.** |
| `bind` | `(workspaceDir) => BoundApi` | Sugar for calling all verbs with the same workspaceDir. Stateless: every method still runs `readState -> plan -> applyPlan`. |

### The server value

Every mutating call takes an `McpServer` value:

```ts
interface McpServer {
  name: string          // manifest key + entry name in the agent's config file
  spec: McpServerSpec   // how the server is invoked (stdio | sse | http)
}

type McpServerSpec =
  | { transport: 'stdio'; command: string; args?: string[]; env?: Record<string, string> }
  | { transport: 'sse';   url: string; headers?: Record<string, string> }
  | { transport: 'http';  url: string; headers?: Record<string, string> }
```

The `name` is trimmed before being used as the manifest key. `link({server: {name: '  gh  ', ...}})` persists under `'gh'`.

### Errors

Every error extends `McpManagerError`. `instanceof` checks are safe across module boundaries.

| Error | Thrown by | Meaning |
|---|---|---|
| `AgentNotSupportedError` | any verb | Agent id not in the catalog. |
| `ForeignEntryError` | `link` | On-disk entry under the target name was not put there by the manifest. Pass `allowOverwrite: true` to take ownership. |
| `UnsupportedTransportError` | `link` | Transport not accepted by this agent at this scope. Details include the accepted set and a per-agent hint. |
| `InvalidServerSpecError` | `link` | Server has an empty name, or the spec is missing required fields (empty command, empty url, unknown transport). |
| `UnresolvedConfigPathError` | any verb that needs the on-disk path | Cannot resolve the agent's config file on this OS (e.g., project scope requested without `projectRoot`, env vars unset). |
| `AgentNotInstalledError` | `link` | Neither the agent's config file nor its parent directory exists on disk. The agent has either not been installed or has been installed but never launched. Fields: `.agent`, `.configPath`, `.parentDir`. Precheck with `isInstalled({agents})` to avoid the throw. |

Note: v0.0.3's `ServerNotFoundError` no longer applies to `link`, since the server is passed in on every call.

### Return summaries

Every mutating verb returns a summary describing what actually happened:

```ts
interface LinkPlanSummary {
  serverName: string
  agent: AgentId
  scope: AgentScope
  created: boolean           // true if no prior link existed for this agent; false if we replaced one
  overwroteForeign: boolean  // true if allowOverwrite: true replaced an unmanaged entry
}

interface UnlinkPlanSummary {
  serverName: string
  agent: AgentId
  scope: AgentScope
  removed: boolean           // true if there was actually a link to remove
}

interface DisconnectPlanSummary {
  serverName: string
  agent: AgentId
  scope: AgentScope
  unlinked: boolean          // true if we removed a link record
  removedManifest: boolean   // true if we dropped the manifest entry (last link + removeIfLast)
}

interface RemovePlanSummary {
  serverName: string
  unlinkedAgents: AgentId[]  // every agent whose config file was rewritten
  removedManifest: boolean
}
```

## `rescan`: detecting drift between manifest and disk

The library maintains its own workspace manifest (`<workspaceDir>/manifest.json`) that records every server you've linked and which agent's config file received it. `rescan` compares that manifest against what's actually on disk and returns a report:

```ts
interface RescanReport {
  verified: ReadonlyArray<{ serverName: string; agent: AgentId; configPath: string }>
  drifted:  ReadonlyArray<{ serverName: string; agent: AgentId; scope: AgentScope; configPath: string; reason: string }>
  missing:  ReadonlyArray<{ serverName: string; agent: AgentId; scope: AgentScope; configPath: string; reason: string }>
}
```

### When to call it

`rescan` is your source of truth whenever the on-disk state may have drifted from what the library last wrote. Common triggers:

- **After a user edits the config file directly.** Someone opens `~/.cursor/mcp.json` in a text editor and deletes an entry. The manifest still thinks it's there.
- **After you change a spec.** `link({server: A})` then `link({server: A_prime, agent: 'gemini'})` where the name is the same but the spec differs. The manifest holds the newest spec; `A`'s previous linkers (cursor, vscode, ...) still have the old spec on disk. `rescan` won't report drift on presence, but see the note below on how to interpret it.
- **On app startup.** UI shells that display "which servers are linked to which agents" should call `rescan` on load, not trust the manifest alone.
- **Before batching writes.** If you're about to `link` a large batch, `rescan` first to detect any pre-existing drift you should surface to the user rather than silently overwriting.

### What each bucket means

- **`verified`**: manifest says agent X has server Y linked to `configPath` Z, AND that file exists AND the emitter finds a matching entry inside it. Everything is aligned.
- **`drifted`**: manifest recorded a link, the file exists, but the emitter can't find an entry under that name. Someone (user, another tool, a partial write) removed the entry from disk. The manifest still remembers the link.
- **`missing`**: the config file itself is gone (either the file doesn't exist on disk, or you didn't include the agent in the readState call). The link record survives.

### Common patterns

**Detect drift and prompt to re-link.** The most common consumer flow:

```ts
const report = await rescan(workspaceDir)
for (const drift of report.drifted) {
  console.log(`${drift.agent}'s ${drift.configPath} is missing '${drift.serverName}'`)
  // Ask the user; on confirm, re-link using the current spec stored in the manifest.
  const servers = await list(workspaceDir)
  const server = servers.find((s) => s.name === drift.serverName)
  if (server) {
    await link(workspaceDir, {
      server: { name: server.name, spec: server.spec },
      agent: drift.agent,
    })
  }
}
```

**Filter by agent.** If you only care about drift for the agents you actively support, pass a filter:

```ts
await rescan(workspaceDir, { agents: ['cursor', 'vscode'] })
```

**Report-only in CI.** `rescan` performs zero writes and returns a plain data value. It's safe to run in a CI check that verifies your dev workspace hasn't drifted:

```ts
const { drifted, missing } = await rescan(workspaceDir)
if (drifted.length + missing.length > 0) {
  console.error('MCP config drift detected:', { drifted, missing })
  process.exit(1)
}
```

### What `rescan` does NOT do

- It does not detect **spec drift** (an entry exists on disk but was written with a different command/url than the manifest's current spec). The current implementation only checks for entry presence. Spec-drift detection is a v0.0.5 candidate; today, you can compare `list(workspaceDir)[i].spec` against a parse of the config file yourself if you need it.
- It does not scan for **unmanaged entries** (entries on disk that the manifest never wrote). v0.0.3 exposed this via `RescanResult.unmanaged`; v0.0.4 does not. This is also a v0.0.5 candidate.
- It does not touch disk. `rescan` is read-only and returns a value; the caller decides what to do about it.

## `isInstalled`: checking agent availability

Before you call `link()`, you probably want to know whether the target agent is actually installed on the user's machine. `isInstalled` answers that in one batch call:

```ts
import { isInstalled } from 'agent-mcp-manager'

const installed = await isInstalled({
  agents: ['cursor', 'claude-code', 'gemini', 'vscode'],
})

if (installed.cursor) {
  await link(workspaceDir, { server, agent: 'cursor' })
}
```

Return shape: `Partial<Record<AgentId, boolean>>`. Only the agents you asked about appear as keys. Duplicates in the input array collapse.

### What "installed" means here

An agent is installed iff **its config file already exists OR the parent directory of that config file exists.** For Cursor that's `~/.cursor/mcp.json` or `~/.cursor/`. For Claude Desktop it's `~/Library/Application Support/Claude/claude_desktop_config.json` or `~/Library/Application Support/Claude/`. If neither exists, the agent either hasn't been installed or has been installed but never launched. Either way, the library can't safely write to it.

This is the same signal `link()` uses to gate `AgentNotInstalledError`. So `isInstalled` is exactly the precheck for `link`: if `installed[agent] === false`, `link({server, agent})` will throw.

### When to call it

- **Gating a UI dropdown**: show only installed agents in a "Choose an agent" list.
- **Filtering a batch before linking**: skip missing agents instead of catching per-call throws.
- **Background health check**: on app startup, warn the user if agents they had linked previously are no longer available.
- **Predicting failures**: any time you want to avoid catching `AgentNotInstalledError` in flow control.

### Worked patterns

**Show only installed agents in a UI dropdown.**

```ts
const supported = listSupportedAgents()
const installed = await isInstalled({ agents: supported })
const shown = supported.filter((a) => installed[a] === true)
// Render `shown` in your UI.
```

**Precheck a batch before linking.**

```ts
const targets = ['cursor', 'claude-code', 'gemini']
const installed = await isInstalled({ agents: targets })
for (const agent of targets) {
  if (installed[agent]) {
    await link(workspaceDir, { server, agent })
  } else {
    showWarning(`${agent} is not installed; skipping.`)
  }
}
```

**Predict what `link` will throw.** Because both use the same signal:

```ts
const installed = await isInstalled({ agents: ['cursor'] })
// installed.cursor === false  =>  link({server, agent: 'cursor'}) will throw AgentNotInstalledError.
```

### `isInstalled` vs `detectInstalledAgents`

`detectInstalledAgents` (already in v0.0.3) returns `AgentInfo[]` with an `installed` flag based on the catalog's `installCheckPaths` (app bundle locations like `/Applications/Cursor.app`). It answers "is the app on disk?" That's a different question from "can the library write MCP config here?"

- Use `isInstalled` when you're about to write config or need to precheck `link`. Same signal as the gate.
- Use `detectInstalledAgents` when you want to know whether the app bundle exists, without necessarily knowing whether the user has launched it. Useful for "we found Cursor in /Applications but you haven't launched it yet; do that first" UX flows.

Both signals will drift apart in cases like unlaunched fresh installs. Pick the one that matches your question.

### `AgentNotInstalledError`

The typed error `link()` throws when the install gate fails. Fields:

```ts
class AgentNotInstalledError extends McpManagerError {
  agent: AgentId
  configPath: string  // the exact path we checked
  parentDir: string   // its parent, also missing
}
```

Handle it around your `link()` call, or use `isInstalled` beforehand to avoid the throw entirely.

## Migration from v0.0.3

If you're on v0.0.3 with `createMcpManager`, you have three paths:

1. **Migrate to the functional API** (recommended). This section gives you the mechanical translations.
2. **Pin v0.0.3** with `"agent-mcp-manager": "^0.0.3"` in your package.json. The v0.0.3 line is unmaintained but functional; it stays on npm.
3. **Wait**. No compat shim is planned. v0.0.5+ builds on the functional surface.

### What changed and why

The v0.0.3 `createMcpManager()` returned an object with a private `manifest` reference that mutated across method calls. This produced a real bug ([#63](https://github.com/DaniAkash/agent-toolkit/issues/63)): the "disconnect one agent" flow required a three-line dance (`unlink` + `listLinks` + conditional `remove`), and any race or logic bug in the caller could silently orphan other agents' link records.

Under v0.0.4:

- **No mutable in-memory manifest.** Every verb reads the manifest from disk at call time, computes a plan, applies it.
- **No pre-registration step.** The v0.0.3 flow was `add(name, spec)` then `link(name, agent)`. That two-step surface let the two calls drift out of sync, allowed manifest ghosts, and made concurrent adds silently clobber each other. v0.0.4 collapses this into one `link({server, agent})` call. The server is caller-owned data.
- **Every operation composes.** `readState → plan* → applyPlan`. You can inspect the plan before writing.
- **`disconnect()` is one primitive.** It reads state, unlinks one agent, drops the manifest entry only if no other agents remain linked. Never touches other agents' config files. The #63 bug is structurally impossible.
- **The workspace manifest schema on disk is unchanged.** If you have a `manifest.json` written by v0.0.3, v0.0.4 reads it without migration.

### Migration table

| v0.0.3 (class API) | v0.0.4 (functional API) |
|---|---|
| `const mgr = createMcpManager({ workspaceDir })` | `const mgr = bind(workspaceDir)` (optional; the raw verbs also work) |
| `await mgr.add({ name, spec })`<br>`await mgr.link({ serverName: name, agent })` | `const server = { name, spec }`<br>`await link(workspaceDir, { server, agent })` |
| `await mgr.link({ serverName, agent, configPath })` | `await link(workspaceDir, { server, agent, configPath })` |
| `await mgr.link({ serverName, agent, allowOverwrite: true })` | `await link(workspaceDir, { server, agent, allowOverwrite: true })` |
| `await mgr.unlink({ serverName, agent })` | `await unlink(workspaceDir, { serverName, agent })` |
| `await mgr.remove({ serverName })` | `await remove(workspaceDir, { serverName })` |
| `await mgr.remove({ serverName, unlinkFirst: false })` | `await remove(workspaceDir, { serverName, unlinkFirst: false })` |
| `await mgr.listServers()` | `await list(workspaceDir)` |
| `await mgr.listLinks()` | `await listLinks(workspaceDir)` |
| `await mgr.listLinks({ agents, serverNames })` | `await listLinks(workspaceDir, { agents, serverNames })` |
| `await mgr.rescan()` | `await rescan(workspaceDir)` (see the [rescan section](#rescan-detecting-drift-between-manifest-and-disk) for report changes) |

**No more `addServer`.** v0.0.3 required two calls to link a server for the first time: `mgr.add(...)` then `mgr.link(...)`. v0.0.4 does both in one `link({server, ...})`. Just build the server value inline and hand it to `link`. If you'd been storing the result of `mgr.add()` to reuse in later `mgr.link` calls, replace that with a caller-owned `const server = {name, spec}` variable.

### The disconnect pattern (the #63 fix)

If your v0.0.3 code has this pattern:

```ts
// v0.0.3 (buggy: can orphan other agents' links)
await mgr.unlink({ serverName, agent })
const links = await mgr.listLinks({ serverNames: [serverName] })
if (links.length === 0) {
  await mgr.remove({ serverName })
}
```

Replace it with a single call:

```ts
// v0.0.4 (structural fix)
await disconnect(workspaceDir, {
  serverName,
  agent,
  removeIfLast: true,  // default is true; can pass false to keep the entry
})
```

The v0.0.4 `disconnect` reads the manifest once, computes the post-unlink links map, and only drops the manifest entry when that map is empty. It never touches any other agent's config file. If two callers race, each reads a fresh manifest at call time; the last writer wins on the manifest, and neither ever touches an unrelated agent's config.

### Manager options

v0.0.3 accepted per-manager configuration through `McpManagerOptions`. Those knobs are now per-call:

| v0.0.3 `McpManagerOptions` field | v0.0.4 equivalent |
|---|---|
| `workspaceDir` | The first positional argument to every verb (or `bind(workspaceDir)` for sugar) |
| `scope` | Pass `scope: 'system' \| 'project'` per call |
| `projectRoot` | Pass `projectRoot: string` per call (required when `scope: 'project'`) |
| `agentConfigPaths` | Pass `configPath: string` per call to override the resolved path for that call |

Rationale: v0.0.3's per-manager `agentConfigPaths` map applied to every method call, which made mixed-scope operations awkward. v0.0.4's per-call `configPath` is one field with the same effect and no hidden state.

### Return shape diffs

- **`listServers` → `list`**. v0.0.3 returned `InstalledServer[]`; v0.0.4 `list(workspaceDir)` returns `ManifestServerEntry[]`. Same fields (`{name, spec, addedAt, links}`), different type name. Existing consumer code that reads `.name`, `.spec`, `.addedAt`, `.links` needs no change.
- **`listLinks`**. v0.0.3 returned `McpServerLink[]` with optional `drifted` / `broken` / `unmanaged` flags. v0.0.4 `listLinks(workspaceDir)` returns `ListedLink[]` with just `{serverName, agent, configPath}`. Drift flags moved to `rescan()`, which now returns a dedicated `{verified, drifted, missing}` report.
- **`rescan`**. v0.0.3 `RescanResult` had four buckets: `verified` / `drifted` / `broken` / `unmanaged`. v0.0.4 `RescanReport` has three: `verified` / `drifted` / `missing`. The v0.0.3 `broken` bucket is now folded into `missing` (a manifest link with no on-disk entry). v0.0.3's `unmanaged` bucket (on-disk entries with no manifest record) is not currently scanned; that's a v0.0.5 candidate.

### New capabilities in v0.0.4

Available today; no equivalent in v0.0.3:

- **Dry-run.** Import from `agent-mcp-manager/lowlevel` and call `planLink` / `planDisconnect` / etc. without applying. Inspect `plan.ops` (the exact file writes) and `plan.nextManifest` (the manifest snapshot) before deciding to apply. Example under "Dry-run and batching" below.
- **Batching.** Run multiple planner calls against a single `State` snapshot, concatenate the `ops`, and apply them all in one pass with `applyPlan`.
- **Install-status detection.** `isInstalled({agents})` batch-checks whether each agent is available on the current machine, and `link()` throws `AgentNotInstalledError` (instead of silently creating a ghost config) when the target isn't installed. See the [isInstalled section](#isinstalled-checking-agent-availability).
- **16 additional clients.** `cline`, `opencode`, `goose`, `kiro`, `windsurf`, `witsy`, `roocode`, `enconvo`, `boltai`, `amazon-bedrock`, `amazonq`, `tome`, `librechat`, `antigravity`, `trae`, `vscode-insiders`.

### Known behavioral differences

- **One-step link.** v0.0.3 required `mgr.add()` before `mgr.link()`. v0.0.4 `link({server, agent})` does both. The manifest server entry is upserted as a side-effect of the link.
- **Last-write-wins on the manifest spec.** If you call `link({server: A, agent: 'cursor'})` and later `link({server: A', agent: 'gemini'})` where `A.name === A'.name` but the specs differ, the manifest's spec updates to `A'`. Cursor's config still holds `A` on disk (stale) until you re-link cursor. Use `rescan()` to detect this class of drift.
- **Trimmed names.** `link({server: {name: '  gh  ', ...}})` persists the trimmed key `'gh'`. v0.0.3 persisted the untrimmed value.
- **Idempotent re-links skip the config write.** `link()` no longer touches mtime when the resulting content is identical. IDE file-watchers (Cursor, VS Code) no longer reload on idempotent re-runs.
- **`exists: true` for empty files.** `readState` from `/lowlevel` returns `AgentFileState.exists = true` for existing empty files; v0.0.3 conflated existence with non-emptiness.
- **`unlink` uses the manifest-recorded configPath.** If you `link({ configPath: X })` in v0.0.3 and later `unlink()` without a `configPath`, v0.0.3 rewrote the OS-default path (potentially skipping the file that had the entry). v0.0.4 looks up the recorded path from the manifest first.

### If you consumed `McpManager` as a type

v0.0.3 exported the `McpManager` interface for consumers to store the manager instance in class fields or React refs. v0.0.4 has no equivalent because the verbs are free functions. Two options:

```ts
// Option A: store the workspaceDir, call the free functions on demand.
class MyApp {
  private workspaceDir = '~/.myapp/mcp'
  async link(server: McpServer, agent: AgentId) {
    await link(this.workspaceDir, { server, agent })
  }
}

// Option B: store a BoundApi.
import { bind, type BoundApi, type McpServer, type AgentId } from 'agent-mcp-manager'
class MyApp {
  private mcp: BoundApi = bind('~/.myapp/mcp')
  async link(server: McpServer, agent: AgentId) {
    await this.mcp.link({ server, agent })
  }
}
```

Both are stateless: every method call re-reads the manifest from disk.

## Dry-run and batching

Import from `agent-mcp-manager/lowlevel` for the pure planner primitives plus `readState` and `applyPlan`. Every verb returns a `Plan` you can inspect before touching disk.

```ts
import {
  readState,
  planLink,
  planDisconnect,
  applyPlan,
} from 'agent-mcp-manager/lowlevel'

const state = await readState(workspaceDir, ['cursor', 'gemini'])

// Compute both plans against the SAME state snapshot.
const linkPlan = planLink(
  state,
  { server: { name: 'gh', spec: { transport: 'stdio', command: 'gh-mcp' } }, agent: 'cursor' },
  new Date().toISOString(),
)
const disconnectPlan = planDisconnect(state, { serverName: 'old', agent: 'gemini' })

// Inspect before writing.
console.log('link ops:', linkPlan.ops)
console.log('disconnect ops:', disconnectPlan.ops)

// Apply when ready. Each plan is independent; combined ops write once.
await applyPlan({
  ops: [...linkPlan.ops, ...disconnectPlan.ops],
  nextManifest: disconnectPlan.nextManifest,
})
```

`FsOp` is a discriminated union:

```ts
type FsOp =
  | { kind: 'writeFile'; path: string; content: string; ensureDir?: boolean }
  | { kind: 'removeFile'; path: string }
```

Every write goes through atomic `<file>.tmp + rename`.

## Supported clients

23 clients ship in v0.0.4 with hand-authored per-client shape declarations. See `src/_catalog/client-configs.ts` for the source of truth, including each entry's citation URLs.

| Established | Well-documented | Additional |
|---|---|---|
| claude-desktop | cline | amazon-bedrock |
| claude-code | opencode | amazonq |
| cursor | goose | antigravity |
| vscode | kiro | boltai |
| vscode-insiders | windsurf | enconvo |
| gemini | witsy | librechat |
| codex | roocode | tome |
| zed |  | trae |

Every catalog entry has:

- A first-party MCP docs URL as its primary source.
- A Smithery cross-check URL (design reference, AGPL-3.0; see [`THIRD_PARTY_NOTICES.md`](./THIRD_PARTY_NOTICES.md)).
- An ISO `verified` date. The catalog validator rejects any entry more than 12 months stale.

## Transport support and safety guarantees

Each client declares which transports it accepts (stdio / sse / http). Writing an entry with an unsupported transport throws `UnsupportedTransportError` **before** any file write. Concrete examples:

- Claude Desktop is stdio-only; an HTTP spec throws.
- Claude Code project scope (`.mcp.json`) is stdio-only; system scope (`~/.claude.json`) accepts all three.
- Codex accepts stdio and streamable HTTP but not SSE.

On disk:

- **Atomic writes.** Every edit goes through `<file>.tmp + rename`. A crashed process never leaves a half-written config file.
- **Foreign-entry protection.** `link` throws `ForeignEntryError` when an on-disk entry under the target name was not put there by the manifest. Pass `allowOverwrite: true` to take ownership.
- **Structural protection against orphaning.** `disconnect` computes its ops from the manifest's links map. Under this shape, disconnecting one agent from a server that four others share never touches the four others' config files. The v0.0.3 class API had a bug (issue #63) where `remove` blew away shared manifest entries. That bug is structurally impossible under the FP API.
- **Idempotent writes.** Re-linking a server that's already correctly present skips the file write entirely. Editors watching the config file do not reload.
- **Install-status gate.** `link()` refuses to create a ghost config directory for an agent whose config path is under a non-existent parent. The typed `AgentNotInstalledError` carries the agent id, the config path we checked, and the parent directory so consumers can surface an actionable prompt.

## Types

Everything you need is at the package root or under `agent-mcp-manager/lowlevel`. Import paths:

```ts
import type {
  McpServer,              // { name, spec }
  McpServerSpec,          // stdio | sse | http
  McpStdioSpec,
  McpHttpSpec,
  McpSseSpec,
  AgentId,                // union of 23 client ids
  AgentScope,             // 'system' | 'project'
  ServerManifest,         // on-disk schema
  ManifestServerEntry,
  ManifestLinkEntry,
  LinkPlanSummary,
  UnlinkPlanSummary,
  DisconnectPlanSummary,
  RemovePlanSummary,
  RescanReport,
  IsInstalledInput,
  IsInstalledResult,      // Partial<Record<AgentId, boolean>>
  BoundApi,
} from 'agent-mcp-manager'

import { AgentNotInstalledError } from 'agent-mcp-manager'  // thrown by link()

import type {
  State, Plan, FsOp,
  LinkInput,
  DisconnectInput,
  // ... every planner input and result summary
} from 'agent-mcp-manager/lowlevel'
```

## Contributing

Author under MIT. Every catalog entry needs a first-party docs URL and an ISO `verified` date; the validator enforces both. Test files live next to the source folders they exercise.

License: MIT. See [`THIRD_PARTY_NOTICES.md`](./THIRD_PARTY_NOTICES.md) for research references.
