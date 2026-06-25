/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { Browser } from '@browseros/browser-core/browser'
import type { ProtocolApi } from '@browseros/cdp-protocol/protocol-api'
import { SCREENCAST_LIMITS } from '@browseros/shared/constants/limits'
import type { WSContext } from 'hono/ws'
import { logger } from '../../../lib/logger'

export interface ScreencastFrameMessage {
  type: 'frame'
  data: string
  metadata: {
    timestamp?: number
    deviceWidth?: number
    deviceHeight?: number
    offsetTop?: number
    pageScaleFactor?: number
    scrollOffsetX?: number
    scrollOffsetY?: number
  }
}

export interface ScreencastStatusMessage {
  type: 'status'
  status: 'connected' | 'detached'
  windowId: number
  pageId?: number
  url?: string
}

export type ScreencastOutboundMessage =
  | ScreencastFrameMessage
  | ScreencastStatusMessage

export type Subscriber = WSContext<unknown>

// At most one active screencast across the whole BrowserOS instance.
// A new subscribe displaces the prior one — so frame events from
// different targets can't cross-talk on the same WS connection.
interface ScreencastSession {
  targetId: string
  cdpSession: ProtocolApi
  ws: Subscriber
  unsubscribeFrame: () => void
}

export interface SubscribeHandle {
  targetId: string
}

const WS_OPEN: 1 = 1

export class ScreencastManager {
  private active: ScreencastSession | null = null

  constructor(private readonly browser: Browser) {}

  async subscribe(
    windowId: number,
    pageId: number | null,
    ws: Subscriber,
  ): Promise<SubscribeHandle> {
    let resolved: { targetId: string; session: ProtocolApi; url: string }
    try {
      resolved =
        pageId === null
          ? await this.browser.getActivePageForWindow(windowId)
          : await this.browser.getPageSession(pageId)
    } catch (err) {
      // Stale pageId or dead windowId. Send `detached` rather than
      // letting the route close with 1011, which would trigger an
      // EventSource reconnect spiral on the renderer.
      logger.warn('screencast subscribe could not resolve', {
        windowId,
        pageId,
        error: err instanceof Error ? err.message : String(err),
      })
      this.send(ws, {
        type: 'status',
        status: 'detached',
        windowId,
        pageId: pageId ?? undefined,
      })
      return { targetId: '' }
    }

    // The route's onClose can fire while resolve() was awaiting.
    if (ws.readyState !== WS_OPEN) {
      return { targetId: resolved.targetId }
    }

    await this.tearDown('replaced')

    const session: ScreencastSession = {
      targetId: resolved.targetId,
      cdpSession: resolved.session,
      ws,
      unsubscribeFrame: () => undefined,
    }
    session.unsubscribeFrame = resolved.session.Page.on(
      'screencastFrame',
      (params) => {
        this.send(ws, {
          type: 'frame',
          data: params.data,
          metadata: {
            timestamp: params.metadata.timestamp,
            deviceWidth: params.metadata.deviceWidth,
            deviceHeight: params.metadata.deviceHeight,
            offsetTop: params.metadata.offsetTop,
            pageScaleFactor: params.metadata.pageScaleFactor,
            scrollOffsetX: params.metadata.scrollOffsetX,
            scrollOffsetY: params.metadata.scrollOffsetY,
          },
        })
        resolved.session.Page.screencastFrameAck({
          sessionId: params.sessionId,
        }).catch((err) => {
          logger.warn('screencastFrameAck failed', {
            targetId: resolved.targetId,
            error: err instanceof Error ? err.message : String(err),
          })
        })
      },
    )
    this.active = session

    // Backgrounded tabs don't composite — startScreencast attaches but
    // emits zero frames until something invalidates the surface.
    // bringToFront foregrounds the tab in its window so the compositor
    // wakes. setWebLifecycleState alone is not enough.
    await resolved.session.Page.bringToFront().catch((err) => {
      logger.warn('bringToFront failed', {
        targetId: resolved.targetId,
        error: err instanceof Error ? err.message : String(err),
      })
    })

    // `connected` is sent after bringToFront so it doubles as the
    // focus-restore signal for the agent-company SSE proxy — see
    // screencast-proxy.ts.
    this.send(ws, {
      type: 'status',
      status: 'connected',
      windowId,
      pageId: pageId ?? undefined,
      url: resolved.url,
    })

    await resolved.session.Page.startScreencast({
      format: 'jpeg',
      quality: SCREENCAST_LIMITS.DEFAULT_JPEG_QUALITY,
      everyNthFrame: SCREENCAST_LIMITS.EVERY_NTH_FRAME,
      maxWidth: SCREENCAST_LIMITS.MAX_WIDTH,
      maxHeight: SCREENCAST_LIMITS.MAX_HEIGHT,
    })

    return { targetId: resolved.targetId }
  }

  unsubscribe(handle: SubscribeHandle, ws: Subscriber): void {
    if (!this.active) return
    if (this.active.ws !== ws) return
    if (this.active.targetId !== handle.targetId) return
    void this.tearDown('unsubscribed')
  }

  private async tearDown(reason: 'replaced' | 'unsubscribed'): Promise<void> {
    const session = this.active
    if (!session) return
    this.active = null
    session.unsubscribeFrame()
    if (reason === 'replaced') {
      this.send(session.ws, {
        type: 'status',
        status: 'detached',
        windowId: 0,
      })
    }
    try {
      await session.cdpSession.Page.stopScreencast()
    } catch (err) {
      // Target may already be gone (tab closed, navigated). Best-effort.
      logger.warn('stopScreencast threw', {
        targetId: session.targetId,
        error: err instanceof Error ? err.message : String(err),
      })
    }
  }

  private send(ws: Subscriber, message: ScreencastOutboundMessage): void {
    if (ws.readyState !== WS_OPEN) return
    try {
      ws.send(JSON.stringify(message))
    } catch {
      // Best-effort.
    }
  }
}
