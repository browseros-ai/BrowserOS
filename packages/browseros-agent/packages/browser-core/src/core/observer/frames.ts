import type { ProtocolApi } from '@browseros/cdp-protocol/protocol-api'
import type { CdpConnection, FrameId, SessionId } from '../connection'

interface AttachedParams {
  sessionId: SessionId
  targetInfo?: { type?: string; targetId: string }
  waitingForDebugger?: boolean
}

interface DetachedParams {
  sessionId: SessionId
}

export interface FrameTarget {
  session: ProtocolApi
  axParams: { frameId?: FrameId }
}

/**
 * Tracks out-of-process iframe (OOPIF) sessions via flat auto-attach and resolves any frame to the
 * right CDP session + params. OOPIF sessions are keyed globally by frameId — Chrome reuses the
 * frameId as the OOPIF targetId, which is globally unique — so the owning page never has to be
 * threaded through. Same-origin frames are reached through their page session with a frameId param.
 * Mirrors agent-browser's resolve_ax_session / resolve_frame_session split, unified here.
 */
export class FrameRegistry {
  private readonly oopifSessions = new Map<FrameId, SessionId>()
  private readonly pageSessions = new Map<number, SessionId>()

  constructor(private readonly cdp: CdpConnection) {
    this.cdp.Target.on('attachedToTarget', (params) => {
      void this.onAttached(params as AttachedParams)
    })
    this.cdp.Target.on('detachedFromTarget', (params) => {
      this.onDetached(params as DetachedParams)
    })
  }

  /** Record a page's session and enable cross-origin iframe attachment on it. */
  async registerPage(
    pageSession: ProtocolApi,
    pageId: number,
    sessionId: SessionId,
  ): Promise<void> {
    this.pageSessions.set(pageId, sessionId)
    await pageSession.Target.setAutoAttach({
      autoAttach: true,
      waitForDebuggerOnStart: false,
      flatten: true,
    }).catch(() => {})
  }

  /**
   * main frame → page session, no frameId. cross-origin OOPIF → its dedicated session, no frameId.
   * same-origin iframe → page session WITH a frameId param.
   */
  resolveFrameTarget(
    pageId: number,
    frameId: FrameId | undefined,
  ): FrameTarget {
    const pageSessionId = this.pageSessions.get(pageId)
    if (pageSessionId === undefined) {
      throw new Error(`Page ${pageId} has no attached session`)
    }
    if (frameId === undefined) {
      return { session: this.cdp.session(pageSessionId), axParams: {} }
    }
    const oopif = this.oopifSessions.get(frameId)
    if (oopif) {
      return { session: this.cdp.session(oopif), axParams: {} }
    }
    return { session: this.cdp.session(pageSessionId), axParams: { frameId } }
  }

  private async onAttached(params: AttachedParams): Promise<void> {
    if (params.targetInfo?.type !== 'iframe') return
    this.oopifSessions.set(params.targetInfo.targetId, params.sessionId)
    const session = this.cdp.session(params.sessionId)
    if (params.waitingForDebugger) {
      await session.Runtime.runIfWaitingForDebugger().catch(() => {})
    }
    await session.DOM.enable().catch(() => {})
    await session.Accessibility.enable().catch(() => {})
  }

  private onDetached(params: DetachedParams): void {
    for (const [frameId, sessionId] of this.oopifSessions) {
      if (sessionId === params.sessionId) this.oopifSessions.delete(frameId)
    }
  }
}
