/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Per-agent ownership ledger for browser pages. Populated by
 * successful `tabs new` dispatches and drained by `tabs close`.
 * Two indices stay coherent:
 *   - `records: Map<agentId, Set<pageId>>` powers `ownedBy(agentId)`.
 *   - `owners:  Map<pageId, agentId>`      powers `resolveOwner(pageId)`.
 * `markOpened` writes both; `markClosed` and `forgetAgent` remove
 * from both. The `owners.get(pageId) === agentId` guard on removal
 * handles the (defensive) case of two agents claiming the same
 * pageId: only the recorded owner clears its entry.
 *
 * The cockpit's `tabs list` handler uses `resolveOwner` to classify
 * every open page as `mine` (owner === caller), `user` (no owner),
 * or `other-agent` (owner !== caller). Page-targeted tools still
 * gate on `ownedBy` in register.ts for cross-ownership rejection.
 *
 * Different lifecycle than `tabActivityRegistry`: that one is
 * targetId-keyed and prunes on a 30s window for the /tabs/activity
 * UI. This one is agentId-keyed and lives until the session ends
 * (dropped by cleanupSessionState).
 */

export interface AgentTabsRegistry {
  markOpened(agentId: string, pageId: number): void
  markClosed(agentId: string, pageId: number): void
  /**
   * Returns the set of page ids owned by this agent. NEVER null;
   * unknown agents get an empty set so callers can uniformly do
   * `owned.has(page)` without null-checking.
   */
  ownedBy(agentId: string): ReadonlySet<number>
  /**
   * Returns the agentId currently listed as owner of `pageId`, or
   * null when the page has no agent owner (operator-opened tab).
   * O(1). Used by the `tabs list` handler to classify every page
   * by ownership without walking every agent's ownedBy set.
   */
  resolveOwner(pageId: number): string | null
  /**
   * Drops all pages tracked for an agent. Called from
   * `cleanupSessionState` so the next session for the same agentId
   * starts empty.
   */
  forgetAgent(agentId: string): void
  /**
   * First-capture policy for the audit screenshot fallback. Returns
   * true iff this agent has already had a screenshot written for
   * this pageId within the current session. `services/screenshots.ts`
   * uses this to give every tab exactly one visual anchor even when
   * every dispatch is a read-only tool: the first successful
   * dispatch on a tab writes; subsequent read-only dispatches on
   * the same tab skip. Different agents on the same pageId are
   * tracked independently; each gets its own first-capture write.
   */
  hasFirstCapture(agentId: string, pageId: number): boolean
  /**
   * Records that a screenshot was written for this (agent, page)
   * pair. Called from `persistScreenshot` after a successful write
   * (both tool-result and cache-fallback branches).
   */
  markFirstCaptureDone(agentId: string, pageId: number): void
  // Test-only escape hatches.
  size(): number
  clear(): void
}

const EMPTY: ReadonlySet<number> = new Set<number>()

export function createAgentTabsRegistry(): AgentTabsRegistry {
  const records = new Map<string, Set<number>>()
  const owners = new Map<number, string>()
  const firstCaptures = new Map<string, Set<number>>()
  return {
    markOpened(agentId, pageId) {
      let set = records.get(agentId)
      if (!set) {
        set = new Set()
        records.set(agentId, set)
      }
      set.add(pageId)
      owners.set(pageId, agentId)
    },
    markClosed(agentId, pageId) {
      const set = records.get(agentId)
      if (!set) return
      set.delete(pageId)
      if (owners.get(pageId) === agentId) owners.delete(pageId)
      if (set.size === 0) records.delete(agentId)
    },
    ownedBy(agentId) {
      return records.get(agentId) ?? EMPTY
    },
    resolveOwner(pageId) {
      return owners.get(pageId) ?? null
    },
    forgetAgent(agentId) {
      const set = records.get(agentId)
      if (set) {
        for (const pageId of set) {
          if (owners.get(pageId) === agentId) owners.delete(pageId)
        }
        records.delete(agentId)
      }
      firstCaptures.delete(agentId)
    },
    hasFirstCapture(agentId, pageId) {
      return firstCaptures.get(agentId)?.has(pageId) ?? false
    },
    markFirstCaptureDone(agentId, pageId) {
      let set = firstCaptures.get(agentId)
      if (!set) {
        set = new Set()
        firstCaptures.set(agentId, set)
      }
      set.add(pageId)
    },
    size() {
      return records.size
    },
    clear() {
      records.clear()
      owners.clear()
      firstCaptures.clear()
    },
  }
}

/** Process-wide singleton consumed by register.ts + single-server.ts. */
export const agentTabs = createAgentTabsRegistry()
