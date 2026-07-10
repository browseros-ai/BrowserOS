/**
 * Functional public API. Each verb composes readState -> plan* ->
 * applyPlan against the same workspaceDir. Zero shared mutable state
 * between calls; the manifest is a value read at the boundary.
 *
 * bind(workspaceDir) is stateless sugar for consumers who always pass
 * the same workspaceDir: each method call still goes through the full
 * readState + plan + applyPlan pipeline.
 *
 * For dry-run / batch composition, import from './lowlevel' instead
 * and use the pure planner functions directly.
 */

import { dirname } from 'node:path'

import { pathExists } from './_internal/paths'
import { resolveAgentMcpConfigPath } from './agents'
import { UnresolvedConfigPathError } from './errors'
import { applyPlan, readState } from './io/index'
import {
  planDisconnect,
  planLink,
  planRemove,
  planRescan,
  planUnlink,
} from './planner/planner'
import type {
  DisconnectPlanSummary,
  LinkPlanSummary,
  RemovePlanSummary,
  RescanReport,
  UnlinkPlanSummary,
} from './planner/types'
import type {
  AgentId,
  AgentScope,
  ManifestServerEntry,
  McpServer,
} from './types'

function nowIso(): string {
  return new Date().toISOString()
}

export interface LinkInputAPI {
  /**
   * The server value to link. `name` becomes the manifest key; `spec`
   * is written to the agent's config file and stored on the manifest
   * server entry (last-write-wins across links).
   */
  server: McpServer
  agent: AgentId
  scope?: AgentScope
  projectRoot?: string
  configPath?: string
  allowOverwrite?: boolean
}

export async function link(
  workspaceDir: string,
  input: LinkInputAPI,
): Promise<LinkPlanSummary> {
  const state = await readState(workspaceDir, [input.agent], {
    scope: input.scope,
    projectRoot: input.projectRoot,
    overrides: input.configPath
      ? { [input.agent]: input.configPath }
      : undefined,
  })
  const plan = planLink(state, input, nowIso())
  await applyPlan(plan)
  return {
    serverName: plan.serverName,
    agent: plan.agent,
    scope: plan.scope,
    created: plan.created,
    overwroteForeign: plan.overwroteForeign,
  }
}

export interface UnlinkInputAPI {
  serverName: string
  agent: AgentId
  scope?: AgentScope
  projectRoot?: string
  configPath?: string
}

export async function unlink(
  workspaceDir: string,
  input: UnlinkInputAPI,
): Promise<UnlinkPlanSummary> {
  // Same precedence disconnect/remove use: explicit override first, then
  // whatever configPath the manifest recorded for this link, then the
  // OS-resolved default. Without the manifest lookup, an unlink after a
  // link() with a non-default path would drop the manifest link record
  // but leave the on-disk entry orphaned.
  const initial = await readState(workspaceDir)
  const recorded =
    initial.manifest.servers[input.serverName]?.links[input.agent]?.configPath
  const configPath = input.configPath ?? recorded
  const state = await readState(workspaceDir, [input.agent], {
    scope: input.scope,
    projectRoot: input.projectRoot,
    overrides: configPath ? { [input.agent]: configPath } : undefined,
  })
  const plan = planUnlink(state, input)
  await applyPlan(plan)
  return {
    serverName: plan.serverName,
    agent: plan.agent,
    scope: plan.scope,
    removed: plan.removed,
  }
}

export interface DisconnectInputAPI {
  serverName: string
  agent: AgentId
  scope?: AgentScope
  projectRoot?: string
  removeIfLast?: boolean
}

export async function disconnect(
  workspaceDir: string,
  input: DisconnectInputAPI,
): Promise<DisconnectPlanSummary> {
  // Look up the recorded configPath from the manifest so we read the
  // exact file link() wrote to. Without this, an override at link()
  // time gets lost and disconnect would rewrite the OS-resolved path
  // instead of the one that actually has the entry.
  const initial = await readState(workspaceDir)
  const linkRecord =
    initial.manifest.servers[input.serverName]?.links[input.agent]
  const state = await readState(workspaceDir, [input.agent], {
    scope: input.scope,
    projectRoot: input.projectRoot,
    overrides: linkRecord?.configPath
      ? { [input.agent]: linkRecord.configPath }
      : undefined,
  })
  const plan = planDisconnect(state, input)
  await applyPlan(plan)
  return {
    agent: plan.agent,
    serverName: plan.serverName,
    scope: plan.scope,
    unlinked: plan.unlinked,
    removedManifest: plan.removedManifest,
  }
}

export interface RemoveInputAPI {
  serverName: string
  unlinkFirst?: boolean
}

export async function remove(
  workspaceDir: string,
  input: RemoveInputAPI,
): Promise<RemovePlanSummary> {
  // Read every linked agent's config file at the exact path the
  // manifest records. Same rationale as disconnect(): the link record
  // is the source of truth for where we wrote, not the OS default.
  const manifestState = await readState(workspaceDir)
  const server = manifestState.manifest.servers[input.serverName]
  const linkedAgents = server
    ? (Object.keys(server.links) as AgentId[]).filter((a) => server.links[a])
    : []
  const overrides: Partial<Record<AgentId, string>> = {}
  if (server) {
    for (const agent of linkedAgents) {
      const cp = server.links[agent]?.configPath
      if (cp) overrides[agent] = cp
    }
  }
  const state = await readState(workspaceDir, linkedAgents, { overrides })
  const plan = planRemove(state, input)
  await applyPlan(plan)
  return {
    serverName: plan.serverName,
    unlinkedAgents: plan.unlinkedAgents,
    removedManifest: plan.removedManifest,
  }
}

export async function list(
  workspaceDir: string,
): Promise<ManifestServerEntry[]> {
  const state = await readState(workspaceDir)
  return Object.values(state.manifest.servers)
}

export interface ListedLink {
  serverName: string
  agent: AgentId
  configPath: string
}

export interface ListLinksInputAPI {
  serverNames?: string[]
  agents?: AgentId[]
}

export async function listLinks(
  workspaceDir: string,
  input?: ListLinksInputAPI,
): Promise<ListedLink[]> {
  const state = await readState(workspaceDir)
  const filterServers = input?.serverNames ? new Set(input.serverNames) : null
  const filterAgents = input?.agents ? new Set(input.agents) : null
  const out: ListedLink[] = []
  for (const server of Object.values(state.manifest.servers)) {
    if (filterServers && !filterServers.has(server.name)) continue
    for (const [agentRaw, link] of Object.entries(server.links)) {
      if (!link) continue
      const agent = agentRaw as AgentId
      if (filterAgents && !filterAgents.has(agent)) continue
      out.push({ serverName: server.name, agent, configPath: link.configPath })
    }
  }
  return out
}

export interface RescanInputAPI {
  agents?: AgentId[]
}

export async function rescan(
  workspaceDir: string,
  input?: RescanInputAPI,
): Promise<RescanReport> {
  const manifestState = await readState(workspaceDir)
  const filterAgents = input?.agents
  const referencedAgents = new Set<AgentId>()
  for (const server of Object.values(manifestState.manifest.servers)) {
    for (const agent of Object.keys(server.links)) {
      const id = agent as AgentId
      if (!filterAgents || filterAgents.includes(id)) referencedAgents.add(id)
    }
  }
  const state = await readState(workspaceDir, [...referencedAgents])
  const { rescan: report } = planRescan(state, input)
  return report
}

// -------------------------------------------------------------------
// isInstalled: batch check whether agents can safely receive a link.
//
// "Installed" here means "the library can write MCP config to this
// agent's config path": either the config file already exists on disk
// OR its parent directory does. This is the same signal `link()` uses
// to gate `AgentNotInstalledError`, so `isInstalled` predicts what
// `link()` would throw.
// -------------------------------------------------------------------

export interface IsInstalledInput {
  agents: AgentId[]
  scope?: AgentScope
  projectRoot?: string
}

/**
 * Flat map of agent id to install boolean. Only agents present in
 * `input.agents` appear as keys; consumers who need to iterate know the
 * key set from their own input. Duplicate ids in the input collapse.
 */
export type IsInstalledResult = Partial<Record<AgentId, boolean>>

export async function isInstalled(
  input: IsInstalledInput,
): Promise<IsInstalledResult> {
  const scope = input.scope ?? 'system'
  const out: IsInstalledResult = {}
  for (const agent of input.agents) {
    if (agent in out) continue
    out[agent] = await checkOneInstalled(agent, scope, input.projectRoot)
  }
  return out
}

async function checkOneInstalled(
  agent: AgentId,
  scope: AgentScope,
  projectRoot: string | undefined,
): Promise<boolean> {
  let configPath: string
  try {
    configPath = await resolveAgentMcpConfigPath(agent, scope, projectRoot)
  } catch (err) {
    if (err instanceof UnresolvedConfigPathError) return false
    throw err
  }
  if (await pathExists(configPath)) return true
  if (await pathExists(dirname(configPath))) return true
  return false
}

// -------------------------------------------------------------------
// bind: convenience wrapper for a fixed workspaceDir
// -------------------------------------------------------------------

export interface BoundApi {
  link(input: LinkInputAPI): Promise<LinkPlanSummary>
  unlink(input: UnlinkInputAPI): Promise<UnlinkPlanSummary>
  disconnect(input: DisconnectInputAPI): Promise<DisconnectPlanSummary>
  remove(input: RemoveInputAPI): Promise<RemovePlanSummary>
  list(): Promise<ManifestServerEntry[]>
  listLinks(input?: ListLinksInputAPI): Promise<ListedLink[]>
  rescan(input?: RescanInputAPI): Promise<RescanReport>
  isInstalled(input: IsInstalledInput): Promise<IsInstalledResult>
}

export function bind(workspaceDir: string): BoundApi {
  return {
    link: (input) => link(workspaceDir, input),
    unlink: (input) => unlink(workspaceDir, input),
    disconnect: (input) => disconnect(workspaceDir, input),
    remove: (input) => remove(workspaceDir, input),
    list: () => list(workspaceDir),
    listLinks: (input) => listLinks(workspaceDir, input),
    rescan: (input) => rescan(workspaceDir, input),
    isInstalled: (input) => isInstalled(input),
  }
}
