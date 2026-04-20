export class MonitoringSessionRegistry {
  private readonly activeSessionsByAgent = new Map<string, string>()

  setActive(agentId: string, monitoringSessionId: string): void {
    this.activeSessionsByAgent.set(agentId, monitoringSessionId)
  }

  getActive(agentId: string): string | undefined {
    return this.activeSessionsByAgent.get(agentId)
  }

  getSingleActive():
    | { agentId: string; monitoringSessionId: string }
    | undefined {
    if (this.activeSessionsByAgent.size !== 1) {
      return undefined
    }

    const [agentId, monitoringSessionId] =
      this.activeSessionsByAgent.entries().next().value ?? []

    if (!agentId || !monitoringSessionId) {
      return undefined
    }

    return { agentId, monitoringSessionId }
  }
  clearIfMatches(agentId: string, monitoringSessionId: string): void {
    if (this.activeSessionsByAgent.get(agentId) !== monitoringSessionId) {
      return
    }
    this.activeSessionsByAgent.delete(agentId)
  }
}
