import type { ConnectionState } from '@/modules/api/connections.hooks'

/** Which group each agent was in when the page first loaded, by harness name. */
export type AgentPlacement = ReadonlyMap<string, boolean>

/** Remembers each agent's group as it is now. */
export function placementOf(rows: readonly ConnectionState[]): AgentPlacement {
  return new Map(rows.map((row) => [row.harness, row.installed]))
}

/**
 * Splits agents into Connected and Not connected yet by where they were when
 * the page loaded, not where they are now, so connecting or disconnecting
 * changes a row in place instead of moving it. An agent the page had not
 * seen yet goes by its current state.
 */
export function groupAgents(
  rows: readonly ConnectionState[],
  placement: AgentPlacement | null,
): { connected: ConnectionState[]; notConnected: ConnectionState[] } {
  const wasConnected = (row: ConnectionState) =>
    placement?.get(row.harness) ?? row.installed
  return {
    connected: rows.filter(wasConnected),
    notConnected: rows.filter((row) => !wasConnected(row)),
  }
}
