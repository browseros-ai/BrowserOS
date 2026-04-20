export class MonitoringSessionRegistry {
  private readonly activeSessionsByAgent = new Map<string, string>()

  setActive(agentId: string, monitoringSessionId: string): void {
    this.activeSessionsByAgent.set(agentId, monitoringSessionId)
  }

  getActive(agentId: string): string | undefined {
    return this.activeSessionsByAgent.get(agentId)
  }

  clearIfMatches(agentId: string, monitoringSessionId: string): void {
    if (this.activeSessionsByAgent.get(agentId) !== monitoringSessionId) {
      return
    }
    this.activeSessionsByAgent.delete(agentId)
  }
}
