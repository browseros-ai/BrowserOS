/**
 * Planner types. Pure discriminated unions, no I/O anywhere.
 *
 * State is what the planner sees at the boundary: the workspace
 * manifest plus every agent config file the caller opted in to
 * reading. Plan is the exact list of filesystem ops that would run,
 * with a next-manifest snapshot the caller can inspect before applying.
 *
 * The whole point of these types is that a planner call cannot lie
 * about what it would do. If planDisconnect touches an agent that
 * was not in the input, that agent shows up in `ops`. If planRemove
 * drops a shared manifest entry, the caller sees `nextManifest` before
 * anything is written. The class-based v0.0.3 API could not offer
 * this because state mutated across method calls.
 */

import type { AgentId, AgentScope, McpServer, ServerManifest } from '../types'

/**
 * Snapshot of on-disk state read at the I/O boundary. Every planner
 * consumes this and returns a Plan with the same shape of ops.
 */
export interface State {
  workspaceDir: string
  manifestPath: string
  manifest: ServerManifest
  /** Per-(agent, scope) file view. The planner reads only these; unknown agents are treated as no-op. */
  agents: ReadonlyArray<AgentFileState>
}

export interface AgentFileState {
  agent: AgentId
  scope: AgentScope
  /** Absolute path of the config file we would write to. */
  configPath: string
  /** Raw file contents read at boundary time. Empty string when the file does not exist yet. */
  rawContent: string
  /** True when the file existed on disk when State was read. */
  exists: boolean
  /**
   * True when the parent directory of `configPath` existed on disk at
   * State read time. A false here means the agent is not installed
   * (or has never been launched); `planLink` throws
   * `AgentNotInstalledError` rather than silently `mkdir -p`-ing the
   * directory and creating a ghost config.
   */
  parentExists: boolean
}

/**
 * A single filesystem operation. Applied top-down by applyPlan. Each op
 * is idempotent: running the same plan twice against the same state
 * produces the same on-disk result.
 */
export type FsOp =
  | { kind: 'writeFile'; path: string; content: string; ensureDir?: boolean }
  | { kind: 'removeFile'; path: string }

export interface Plan {
  ops: FsOp[]
  /**
   * The manifest that would land on disk if applyPlan runs to
   * completion. Present in every plan even when ops are empty; the
   * caller can diff against `state.manifest` to preview.
   */
  nextManifest: ServerManifest
}

// -------------------------------------------------------------------
// Planner inputs and per-verb return shapes
// -------------------------------------------------------------------

export interface LinkInput {
  /**
   * Caller-owned server value. `link()` upserts the manifest server
   * entry from `server.spec` (last-write-wins) and adds a link record
   * for the given agent. There is no separate "register a server"
   * step; a server exists in the manifest iff at least one link has
   * been recorded for it.
   */
  server: McpServer
  agent: AgentId
  scope?: AgentScope
  /** Bypass the foreign-entry safety check. Default false. */
  allowOverwrite?: boolean
}

export interface UnlinkInput {
  serverName: string
  agent: AgentId
  scope?: AgentScope
}

export interface DisconnectInput {
  serverName: string
  agent: AgentId
  scope?: AgentScope
  /**
   * When true (default), drop the manifest entry once no agents remain
   * linked to it. False means the manifest keeps the entry with an
   * empty `links` map so future links can re-attach. Never touches
   * the config files of agents other than `agent`.
   */
  removeIfLast?: boolean
}

export interface RemoveInput {
  serverName: string
  /**
   * When true (default), also unlink every currently linked agent
   * before dropping the manifest entry. Every affected agent's config
   * file gets a removeFile op or a rewrite op via the emitter.
   */
  unlinkFirst?: boolean
}

export interface RescanInput {
  agents?: AgentId[]
}

// -------------------------------------------------------------------
// Result shapes that surface alongside a Plan
// -------------------------------------------------------------------

export interface LinkPlanSummary {
  serverName: string
  agent: AgentId
  scope: AgentScope
  /** True if there was no prior link record; false if we replaced one. */
  created: boolean
  /**
   * True when the on-disk config had an entry under this name that the
   * manifest did not put there, and allowOverwrite was true. Callers
   * usually surface this in UI. Never true when allowOverwrite was
   * false; that path throws before returning a plan.
   */
  overwroteForeign: boolean
}

export interface UnlinkPlanSummary {
  serverName: string
  agent: AgentId
  scope: AgentScope
  /** True when there was actually a link to remove. False = no-op. */
  removed: boolean
}

export interface DisconnectPlanSummary {
  agent: AgentId
  serverName: string
  scope: AgentScope
  /** True when we removed a link record from the manifest. */
  unlinked: boolean
  /** True when we dropped the manifest entry entirely (last agent + removeIfLast). */
  removedManifest: boolean
}

export interface RemovePlanSummary {
  serverName: string
  /** Agents whose config files were rewritten to drop the entry. */
  unlinkedAgents: AgentId[]
  /** True when the manifest entry existed and was dropped. */
  removedManifest: boolean
}

// -------------------------------------------------------------------
// Rescan return type (matches the v0.0.3 shape for continuity)
// -------------------------------------------------------------------

export interface RescanEntryDrift {
  serverName: string
  agent: AgentId
  scope: AgentScope
  configPath: string
  reason: string
}

export interface RescanReport {
  verified: ReadonlyArray<{
    serverName: string
    agent: AgentId
    configPath: string
  }>
  drifted: ReadonlyArray<RescanEntryDrift>
  missing: ReadonlyArray<RescanEntryDrift>
}
