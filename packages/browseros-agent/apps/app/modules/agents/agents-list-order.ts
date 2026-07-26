import type { HarnessAgent } from './agent-harness-types'

/**
 * Stable ordering for index-shaped agent surfaces (the `/agents` rail
 * and the chat-screen rail at `/agents/:agentId`). Pinned rows float
 * to the top, then recency desc, with never-used agents falling to
 * the bottom in id-stable order. The rail keeps recency stable so it
 * doesn't reshuffle as turns transition every 5s.
 */
export function orderAgentsByPinThenRecency(
  agents: HarnessAgent[],
): HarnessAgent[] {
  return [...agents].sort((a, b) => {
    const aPinned = a.pinned ?? false
    const bPinned = b.pinned ?? false
    if (aPinned !== bPinned) return aPinned ? -1 : 1

    const aValue = a.lastUsedAt ?? Number.NEGATIVE_INFINITY
    const bValue = b.lastUsedAt ?? Number.NEGATIVE_INFINITY
    if (aValue !== bValue) return bValue - aValue

    return a.id.localeCompare(b.id)
  })
}

/**
 * Same comparator, but operates over arbitrary records that carry
 * `pinned`, `lastUsedAt`, and an `id`-equivalent key. Used by the
 * `/agents` `AgentList` which pivots `AgentListItem` + harness
 * lookup into a sortable shape; both surfaces stay on identical
 * sort semantics through this adapter.
 */
export function compareAgentsByPinThenRecency<
  T extends { pinned: boolean; lastUsedAt: number | null; id: string },
>(a: T, b: T): number {
  if (a.pinned !== b.pinned) return a.pinned ? -1 : 1

  const aValue = a.lastUsedAt ?? Number.NEGATIVE_INFINITY
  const bValue = b.lastUsedAt ?? Number.NEGATIVE_INFINITY
  if (aValue !== bValue) return bValue - aValue

  return a.id.localeCompare(b.id)
}
