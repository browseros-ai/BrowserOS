import type { MonitoringSessionContext } from './types'

interface ActiveMonitoringSession {
  monitoringSessionId: string
  source: MonitoringSessionContext['source']
}

type SessionEndListener = () => void

export class MonitoringSessionRegistry {
  private readonly activeSessionsByAgent = new Map<
    string,
    ActiveMonitoringSession
  >()
  private readonly endListenersByAgent = new Map<
    string,
    Set<SessionEndListener>
  >()

  setActive(
    agentId: string,
    monitoringSessionId: string,
    source: MonitoringSessionContext['source'],
  ): void {
    this.activeSessionsByAgent.set(agentId, { monitoringSessionId, source })
  }

  /**
   * Subscribe to "session ended for this agent" events. The listener fires
   * once per termination — `clearIfMatches` is the only place that drops an
   * active session, so each clear notifies all current listeners. Returns an
   * unsubscribe function. Used by `waitForSessionFree` to gate user-chat
   * sends behind in-flight cron / hook turns without polling.
   */
  onSessionEnd(agentId: string, listener: SessionEndListener): () => void {
    let listeners = this.endListenersByAgent.get(agentId)
    if (!listeners) {
      listeners = new Set()
      this.endListenersByAgent.set(agentId, listeners)
    }
    listeners.add(listener)
    return () => {
      listeners?.delete(listener)
      if (listeners && listeners.size === 0) {
        this.endListenersByAgent.delete(agentId)
      }
    }
  }

  getActive(agentId: string): string | undefined {
    return this.activeSessionsByAgent.get(agentId)?.monitoringSessionId
  }

  resolveForUnattributedToolCalls():
    | { agentId: string; monitoringSessionId: string }
    | undefined {
    const activeSessions = [...this.activeSessionsByAgent.entries()].flatMap(
      ([agentId, session]) =>
        session?.monitoringSessionId
          ? [
              {
                agentId,
                monitoringSessionId: session.monitoringSessionId,
                source: session.source,
              },
            ]
          : [],
    )

    if (activeSessions.length === 1) {
      const [{ agentId, monitoringSessionId }] = activeSessions
      return { agentId, monitoringSessionId }
    }

    const openClawSessions = activeSessions.filter(
      (session) => session.source === 'openclaw-agent-chat',
    )

    if (openClawSessions.length === 1) {
      const [{ agentId, monitoringSessionId }] = openClawSessions
      return { agentId, monitoringSessionId }
    }

    return undefined
  }

  clearIfMatches(agentId: string, monitoringSessionId: string): void {
    if (
      this.activeSessionsByAgent.get(agentId)?.monitoringSessionId !==
      monitoringSessionId
    ) {
      return
    }
    this.activeSessionsByAgent.delete(agentId)
    const listeners = this.endListenersByAgent.get(agentId)
    if (listeners) {
      // Snapshot the set: listeners commonly unsubscribe themselves inside
      // their own callback (one-shot waiters), which would mutate the live
      // set mid-iteration.
      for (const listener of [...listeners]) {
        try {
          listener()
        } catch {}
      }
    }
  }
}
