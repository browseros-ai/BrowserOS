/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Renders the human-help takeover bar on the exact tab an agent is blocked on,
 * so the human can hand control back without leaving the page. The content
 * script never reaches the claw server directly (a page-origin fetch would be
 * cross-origin); it asks the background, which holds host permissions, whether
 * its own tab is the blocked one and relays the hand-back.
 */

import { createShadowRootUi } from 'wxt/utils/content-script-ui/shadow-root'
import { defineContentScript } from 'wxt/utils/define-content-script'
import type {
  TakeoverContext,
  TakeoverPollResponse,
} from '@/modules/takeover/takeover.types'
import {
  createTakeoverBar,
  TAKEOVER_BAR_CSS,
  type TakeoverBar,
} from '@/modules/takeover/takeover-bar'

const POLL_INTERVAL_MS = 3000

export default defineContentScript({
  matches: ['<all_urls>'],
  runAt: 'document_idle',
  allFrames: false,
  async main(ctx) {
    let currentContext: TakeoverContext | null = null

    const ui = await createShadowRootUi<TakeoverBar>(ctx, {
      name: 'browseros-neo-takeover',
      position: 'overlay',
      alignment: 'bottom-left',
      zIndex: 2147483000,
      anchor: 'body',
      css: TAKEOVER_BAR_CSS,
      onMount(container) {
        return createTakeoverBar(container, {
          onHandBack(note) {
            if (!currentContext) return
            void chrome.runtime
              .sendMessage({
                type: 'takeover-resolve',
                sessionId: currentContext.sessionId,
                note: note || undefined,
              })
              .catch(() => {})
          },
        })
      },
      onRemove(mounted) {
        mounted?.destroy()
      },
    })
    ui.mount()

    const poll = async () => {
      // Backgrounded tabs do not need the bar; the human focuses the blocked
      // tab to act, which triggers an immediate poll via visibilitychange.
      if (document.visibilityState !== 'visible') return
      let response: TakeoverPollResponse | undefined
      try {
        response = (await chrome.runtime.sendMessage({
          type: 'takeover-poll',
        })) as TakeoverPollResponse | undefined
      } catch {
        response = undefined
      }
      currentContext = response?.active ? response.context : null
      ui.mounted?.update(currentContext)
    }

    const timer = setInterval(() => void poll(), POLL_INTERVAL_MS)
    ctx.addEventListener(document, 'visibilitychange', () => void poll())
    ctx.onInvalidated(() => {
      clearInterval(timer)
      ui.remove()
    })
    void poll()
  },
})
