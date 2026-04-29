import type { AgentListItem } from './agents-page-types'

/**
 * Display rules for the redesigned agent rows. Pure helpers — no React,
 * no API calls — so they're trivial to unit-test and the row card stays
 * focused on layout.
 */

const UUID_PATTERN =
  /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i

const OC_UUID_PATTERN =
  /^oc-[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i

/**
 * The agent rail used to render whatever the gateway returned for `name`.
 * Post-migration that's frequently the agent's UUID — readable to nobody.
 * Prefer the explicit `name` when it differs meaningfully from the id;
 * otherwise fall back to a short prefix users can recognize on second
 * glance.
 */
export function displayName(agent: AgentListItem): string {
  const name = agent.name?.trim()
  const id = agent.agentId
  if (!name || name === id) {
    if (OC_UUID_PATTERN.test(id)) return id.slice(0, 11) // "oc-XXXXXXXX"
    if (UUID_PATTERN.test(id)) return id.slice(0, 8)
    return id
  }
  return name
}

export function canDelete(agent: AgentListItem): boolean {
  // The gateway's protected `main` agent must not be deletable. The
  // server enforces this too, but disabling the menu item avoids users
  // hitting an opaque 400.
  if (agent.agentId === 'main') return false
  return agent.canDelete
}

/**
 * Rename will be wired to a future `PATCH /agents/:id` endpoint. The
 * legacy `/claw/agents` create flow named the agent on the gateway via
 * the `name` field but the field isn't editable post-create today.
 */
export function canRename(_agent: AgentListItem): boolean {
  return false
}

/**
 * The detail line carries the agent's workspace path. The `detail`
 * field on AgentListItem already holds it for OpenClaw entries
 * (`/home/node/.openclaw/workspace-...`); for harness agents it's the
 * synthetic `<adapter>:main` marker that's not informative — hide it.
 */
export function workspaceLabel(agent: AgentListItem): string | null {
  if (!agent.detail) return null
  if (/^(claude|codex|openclaw):main$/.test(agent.detail)) return null
  return agent.detail
}

const ONE_MINUTE = 60_000
const ONE_HOUR = 60 * ONE_MINUTE
const ONE_DAY = 24 * ONE_HOUR

/**
 * Lightweight relative-time formatter. We don't want to drag in
 * `dayjs/relativeTime` just for a few labels.
 */
export function formatRelativeTime(epochMs: number | null): string {
  if (epochMs === null || !Number.isFinite(epochMs)) return 'never'
  const diff = Math.max(0, Date.now() - epochMs)
  if (diff < ONE_MINUTE) return 'just now'
  if (diff < ONE_HOUR) {
    const m = Math.floor(diff / ONE_MINUTE)
    return `${m} min ago`
  }
  if (diff < ONE_DAY) {
    const h = Math.floor(diff / ONE_HOUR)
    return h === 1 ? '1 hr ago' : `${h} hr ago`
  }
  const d = Math.floor(diff / ONE_DAY)
  return d === 1 ? '1 day ago' : `${d} days ago`
}
