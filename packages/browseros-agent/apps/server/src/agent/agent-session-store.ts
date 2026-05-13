export interface ActiveSession {
  sessionId: string
  agentId: string
  refCount: number
  createdAt: number
  updatedAt: number
  cwd?: string | null
  title?: string | null
  mode: string
  model?: string | null
  turnCount: number
  lastMessagePreview?: string | null
  lastMessageAt?: number | null
  meta?: Record<string, unknown> | null
}

export class AgentSessionStore {
  private sessions = new Map<string, ActiveSession>()

  async openSession(
    agentId: string,
    sessionId: string,
    cwd?: string,
  ): Promise<ActiveSession> {
    const existing = this.sessions.get(sessionId)
    if (existing) {
      existing.refCount++
      existing.updatedAt = Date.now()
      return existing
    }

    const now = Date.now()
    const session: ActiveSession = {
      sessionId,
      agentId,
      refCount: 1,
      createdAt: now,
      updatedAt: now,
      cwd: cwd ?? null,
      title: null,
      mode: 'default',
      model: null,
      turnCount: 0,
      lastMessagePreview: null,
      lastMessageAt: null,
      meta: null,
    }

    this.sessions.set(sessionId, session)
    return session
  }

  async closeSession(sessionId: string): Promise<number> {
    const session = this.sessions.get(sessionId)
    if (!session) {
      return -1
    }

    session.refCount--
    session.updatedAt = Date.now()

    if (session.refCount <= 0) {
      this.sessions.delete(sessionId)
      return 0
    }

    return session.refCount
  }

  async listSessions(
    agentId: string,
    options?: { cursor?: string; limit?: number; search?: string },
  ): Promise<ActiveSession[]> {
    let sessions = Array.from(this.sessions.values())
      .filter((s) => s.agentId === agentId)
      .sort((a, b) => b.updatedAt - a.updatedAt)

    if (options?.search) {
      const query = options.search.toLowerCase()
      sessions = sessions.filter((s) => s.title?.toLowerCase().includes(query))
    }

    if (options?.cursor) {
      const idx = sessions.findIndex((s) => s.sessionId === options.cursor)
      if (idx !== -1) {
        sessions = sessions.slice(idx + 1)
      }
    }

    if (options?.limit) {
      sessions = sessions.slice(0, options.limit)
    }

    return sessions
  }

  async getSessionMeta(sessionId: string): Promise<ActiveSession | null> {
    return this.sessions.get(sessionId) ?? null
  }

  async updateSessionMeta(
    sessionId: string,
    updates: Partial<
      Pick<
        ActiveSession,
        | 'title'
        | 'turnCount'
        | 'lastMessagePreview'
        | 'lastMessageAt'
        | 'mode'
        | 'model'
        | 'meta'
      >
    >,
  ): Promise<ActiveSession | null> {
    const session = this.sessions.get(sessionId)
    if (!session) return null

    Object.assign(session, updates)

    // Auto-set lastMessageAt when preview is provided without explicit timestamp
    if (updates.lastMessagePreview && updates.lastMessageAt === undefined) {
      session.lastMessageAt = Date.now()
    }

    session.updatedAt = Date.now()
    return session
  }
}
