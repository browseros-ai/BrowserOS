/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * In-memory registry of in-flight tool dispatch AbortControllers,
 * keyed by `sessionId`. The MCP dispatch path in
 * `apps/claw-server/src/mcp/register.ts` registers a controller at
 * the start of every tool call and unregisters it in a finally
 * block when the call resolves. The cockpit's Stop button calls
 * `cancelByAgent(agentId)` which walks the identity service for
 * matching sessions and aborts every registered controller.
 *
 * Why per-session sets: a single MCP session can have multiple
 * concurrent tool calls in flight (the SDK allows parallel
 * dispatch). Tracking a Set per session lets us cancel them all
 * with one operator action. The session itself stays open after
 * the abort; the agent's harness sees a normal `isError` result
 * with the cancellation reason and can fire its next tool call
 * immediately.
 *
 * The abort signal here is one half of an `AbortSignal.any(...)`
 * composition in register.ts; the other half is the transport's
 * own signal (from `extra?.signal`, used for client-initiated
 * `notifications/cancelled`). Either side firing aborts the
 * executeTool call cleanly without conflict.
 */

import {
  agentIdentityFromClient,
  type IdentityService,
  identityService,
} from '../lib/mcp-session'

export interface DispatchCancellationService {
  /** Called by register.ts at the start of every tool dispatch. */
  register(sessionId: string, controller: AbortController): void
  /** Called from a try/finally in register.ts when the dispatch resolves. */
  unregister(sessionId: string, controller: AbortController): void
  /**
   * Aborts every active dispatch for this agent record. Returns the
   * count so the route can report
   * whether anything was actually cancelled.
   */
  cancelByAgent(agentId: string, reason: string): number
  /** Test-only escape hatches mirroring tab-activity / identity. */
  size(): number
  clear(): void
}

export interface CreateDispatchCancellationOpts {
  identityService: Pick<IdentityService, 'list'>
}

export const CANCELLATION_REASON = 'Operation cancelled by the User'

export function createDispatchCancellation(
  opts: CreateDispatchCancellationOpts,
): DispatchCancellationService {
  const controllers = new Map<string, Set<AbortController>>()

  return {
    register(sessionId, controller) {
      const existing = controllers.get(sessionId)
      if (existing) {
        existing.add(controller)
        return
      }
      controllers.set(sessionId, new Set([controller]))
    },
    unregister(sessionId, controller) {
      const set = controllers.get(sessionId)
      if (!set) return
      set.delete(controller)
      if (set.size === 0) controllers.delete(sessionId)
    },
    cancelByAgent(agentId, reason) {
      let cancelled = 0
      for (const identity of opts.identityService.list()) {
        const { agentId: candidateAgentId } = agentIdentityFromClient(identity)
        if (candidateAgentId !== agentId) continue
        const set = controllers.get(identity.sessionId)
        if (!set) continue
        for (const controller of set) {
          controller.abort(reason)
          cancelled += 1
        }
      }
      return cancelled
    },
    size() {
      let n = 0
      for (const set of controllers.values()) n += set.size
      return n
    },
    clear() {
      controllers.clear()
    },
  }
}

/** Process-wide singleton consumed by register.ts + the cancel route. */
export const dispatchCancellation = createDispatchCancellation({
  identityService,
})
