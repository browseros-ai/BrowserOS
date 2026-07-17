import type { BrowserOutputFileAccess } from '@browseros/browser-mcp/output-file'
import type { BrowserContext } from '@browseros/shared/schemas/browser-context'
import { logger } from '../lib/logger'
import type { AiSdkAgent } from './ai-sdk-agent'

export interface AgentSession {
  agent: AiSdkAgent
  hiddenPageId?: number
  /** Browser context scoped to the scheduled hidden page. */
  browserContext?: BrowserContext
  /** MCP server names used when the session was created, for change detection. */
  mcpServerKey?: string
  /** Workspace directory when the session was created, for change detection. */
  workingDir?: string
  /**
   * Read-only chat mode when the session was created, for change detection.
   *
   * Required, unlike the other change-detection fields: it is never
   * legitimately absent, and an unstamped session would compare
   * `undefined !== false` and fake a mode change, rebuilding and announcing a
   * switch that never happened. Keeping it required makes the compiler enforce
   * the stamp at every construction site.
   */
  chatMode: boolean
  /** Browser-generated output paths returned during this conversation. */
  outputFileAccess?: BrowserOutputFileAccess
}

export class SessionStore {
  private sessions = new Map<string, AgentSession>()

  get(conversationId: string): AgentSession | undefined {
    return this.sessions.get(conversationId)
  }

  set(conversationId: string, session: AgentSession): void {
    this.sessions.set(conversationId, session)
    logger.info('Session added to store', {
      conversationId,
      totalSessions: this.sessions.size,
    })
  }

  has(conversationId: string): boolean {
    return this.sessions.has(conversationId)
  }

  remove(conversationId: string): boolean {
    const existed = this.sessions.delete(conversationId)
    if (existed) {
      logger.info('Session removed from store (without dispose)', {
        conversationId,
        remainingSessions: this.sessions.size,
      })
    }
    return existed
  }

  async delete(conversationId: string): Promise<boolean> {
    const session = this.sessions.get(conversationId)
    if (!session) return false

    await session.agent.dispose()
    this.sessions.delete(conversationId)
    logger.info('Session deleted', {
      conversationId,
      remainingSessions: this.sessions.size,
    })
    return true
  }

  count(): number {
    return this.sessions.size
  }
}
