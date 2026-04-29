/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * In-memory state machine tracking the live status of every OpenClaw agent
 * session. Acts as the single source of truth for "is agent X running?"
 *
 * Fed exclusively by Gateway WS events — the OpenClawObserver pipes chat
 * broadcast events into this state machine for real-time transitions. The
 * JSONL boot-seed was dropped along with `OpenClawJsonlReader`; consumers
 * see `unknown` for any agent that hasn't emitted an observer event yet.
 */

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
