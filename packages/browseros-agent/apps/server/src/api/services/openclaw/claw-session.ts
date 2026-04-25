/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * In-memory state machine tracking the live status of every OpenClaw agent
 * session. Acts as the single source of truth for "is agent X running?"
 *
 * Two data sources feed it:
 *   1. JSONL files (seed) — on init, reads the latest events for each agent
 *      to infer whether a session is running or idle. This handles the case
 *      where an agent was already mid-task when BrowserOS started.
 *   2. Gateway WS events (live) — the OpenClawObserver pipes chat broadcast
 *      events into this state machine for real-time transitions.
 *
 * Consumers (SSE streams, dashboard endpoint) read from this class and get
 * correct state from the first call — no "unknown" period while waiting for
 * the first WS event.
 */

import { logger } from '../../../lib/logger'
import type { ClawEvent, OpenClawJsonlReader } from './openclaw-jsonl-reader'

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

export type AgentLiveStatus = 'working' | 'idle' | 'error' | 'unknown'

export interface AgentSessionState {
  status: AgentLiveStatus
  sessionKey: string | null
  lastEventAt: number
  currentTool: string | null
  error: string | null
}

export type SessionStateListener = (
  agentId: string,
  state: AgentSessionState,
) => void

// ---------------------------------------------------------------------------
// State machine
// ---------------------------------------------------------------------------

export class ClawSession {
  private readonly states = new Map<string, AgentSessionState>()
  private readonly listeners = new Set<SessionStateListener>()
  private seeded = false

  /**
   * Seed the state machine from JSONL files. Call this once when the
   * gateway becomes ready. For each agent, reads the latest session's
   * events and infers whether the agent is currently working or idle.
   *
   * A session is considered "working" if:
   * - The last message-type event is a user.message (agent hasn't replied yet)
   * - The last event is an agent.tool_use without a matching agent.tool_result
   *
   * Otherwise it's "idle".
   */
  seedFromJsonl(reader: OpenClawJsonlReader): void {
    const agents = reader.listAgents()

    for (const agentId of agents) {
      const sessions = reader.listSessions(agentId)
      if (sessions.length === 0) continue

      const latestSession = sessions[0]
      const events = reader.listBySession(agentId, latestSession.key)
      const state = inferStateFromEvents(events, latestSession.key)

      this.states.set(agentId, state)

      if (state.status === 'working') {
        logger.info('ClawSession seed: agent is working', {
          agentId,
          currentTool: state.currentTool,
        })
      }
    }

    this.seeded = true
    logger.info('ClawSession seeded from JSONL', {
      agentCount: agents.length,
      working: [...this.states.values()].filter((s) => s.status === 'working')
        .length,
    })
  }

  /** Whether seedFromJsonl() has been called. */
  isSeeded(): boolean {
    return this.seeded
  }

  /** Get the current state of an agent. */
  getState(agentId: string): AgentSessionState {
    return (
      this.states.get(agentId) ?? {
        status: 'unknown',
        sessionKey: null,
        lastEventAt: 0,
        currentTool: null,
        error: null,
      }
    )
  }

  /** Get all tracked agent states. */
  getAllStates(): Map<string, AgentSessionState> {
    return this.states
  }

  /**
   * Transition an agent's state. Called by the OpenClawObserver when
   * a chat WS event arrives.
   */
  transition(
    agentId: string,
    status: AgentLiveStatus,
    update: {
      sessionKey?: string | null
      currentTool?: string | null
      error?: string | null
    } = {},
  ): void {
    const prev = this.states.get(agentId)
    const entry: AgentSessionState = {
      status,
      sessionKey: update.sessionKey ?? prev?.sessionKey ?? null,
      lastEventAt: Date.now(),
      currentTool:
        status === 'working'
          ? (update.currentTool ?? prev?.currentTool ?? null)
          : null,
      error: status === 'error' ? (update.error ?? null) : null,
    }

    this.states.set(agentId, entry)

    for (const listener of this.listeners) {
      try {
        listener(agentId, entry)
      } catch {}
    }
  }

  /** Subscribe to state changes. Returns unsubscribe function. */
  onStateChange(listener: SessionStateListener): () => void {
    this.listeners.add(listener)
    return () => this.listeners.delete(listener)
  }
}

// ---------------------------------------------------------------------------
// JSONL state inference
// ---------------------------------------------------------------------------

/**
 * Infer the current session state from JSONL events.
 *
 * The key insight: if the last meaningful event in the JSONL is a
 * user.message with no subsequent agent.message, the agent is still
 * processing (working). Similarly, an agent.tool_use without a matching
 * agent.tool_result means the agent is mid-tool-call.
 *
 * We also check event recency — if the last event was more than 5 minutes
 * ago, we assume the session is idle regardless (handles cases where the
 * agent crashed without writing a final event).
 */
function inferStateFromEvents(
  events: ClawEvent[],
  sessionKey: string,
): AgentSessionState {
  if (events.length === 0) {
    return {
      status: 'idle',
      sessionKey,
      lastEventAt: 0,
      currentTool: null,
      error: null,
    }
  }

  const lastEvent = events[events.length - 1]!
  const lastEventAt = lastEvent.createdAt

  // If the last event is older than 5 minutes, assume idle — the agent
  // likely finished or crashed without writing a final event.
  const STALE_THRESHOLD_MS = 5 * 60 * 1000
  if (Date.now() - lastEventAt > STALE_THRESHOLD_MS) {
    return {
      status: 'idle',
      sessionKey,
      lastEventAt,
      currentTool: null,
      error: null,
    }
  }

  // Walk backward to find the last meaningful event
  let lastUserMessageIdx = -1
  let lastAssistantMessageIdx = -1
  let lastToolUseIdx = -1
  let lastToolResultIdx = -1

  for (let i = events.length - 1; i >= 0; i--) {
    const e = events[i]!
    if (e.type === 'user.message' && lastUserMessageIdx === -1) {
      lastUserMessageIdx = i
    }
    if (e.type === 'agent.message' && lastAssistantMessageIdx === -1) {
      lastAssistantMessageIdx = i
    }
    if (e.type === 'agent.tool_use' && lastToolUseIdx === -1) {
      lastToolUseIdx = i
    }
    if (e.type === 'agent.tool_result' && lastToolResultIdx === -1) {
      lastToolResultIdx = i
    }
    // Stop scanning once we've found all event types
    if (
      lastUserMessageIdx !== -1 &&
      lastAssistantMessageIdx !== -1 &&
      lastToolUseIdx !== -1 &&
      lastToolResultIdx !== -1
    ) {
      break
    }
  }

  // Agent is working if the last user message came AFTER the last
  // assistant message — the agent hasn't replied yet
  if (
    lastUserMessageIdx !== -1 &&
    lastUserMessageIdx > lastAssistantMessageIdx
  ) {
    return {
      status: 'working',
      sessionKey,
      lastEventAt,
      currentTool: null,
      error: null,
    }
  }

  // Agent is working if there's a tool_use without a subsequent tool_result
  if (lastToolUseIdx !== -1 && lastToolUseIdx > lastToolResultIdx) {
    const toolEvent = events[lastToolUseIdx]!
    return {
      status: 'working',
      sessionKey,
      lastEventAt,
      currentTool: toolEvent.toolName ?? null,
      error: null,
    }
  }

  return {
    status: 'idle',
    sessionKey,
    lastEventAt,
    currentTool: null,
    error: null,
  }
}
