/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import * as rrweb from 'rrweb'
import { defineContentScript } from 'wxt/utils/define-content-script'
import { createRecorderBuffer, type RecorderMessage } from '@/modules/recorder'

/** Records each eligible main-frame document from load and relays rrweb batches. */
export default defineContentScript({
  matches: ['<all_urls>'],
  runAt: 'document_start',
  allFrames: false,
  main(ctx) {
    // A stop acknowledgment must follow the worker's durable-outbox commits,
    // including the last batch emitted by buffer.close().
    const pendingPosts = new Set<Promise<boolean>>()
    const buffer = createRecorderBuffer({
      send(ndjson, hasGap) {
        if (ctx.isInvalid) return
        try {
          const post = chrome.runtime
            .sendMessage({
              type: 'recorder-events',
              ndjson,
              hasGap,
            } satisfies RecorderMessage)
            .then((response) => response?.persisted === true)
            .catch((error) => {
              console.warn(
                '[browseros-claw replay] sendMessage to background failed',
                error,
              )
              return false
            })
          pendingPosts.add(post)
          void post.finally(() => pendingPosts.delete(post))
        } catch (error) {
          console.warn('[browseros-claw replay] send threw', error)
        }
      },
      warnDropped(count) {
        console.warn(
          '[browseros-claw replay] dropped',
          count,
          'events under buffer pressure',
        )
      },
    })

    ctx.addEventListener(window, 'pagehide', buffer.flushNow)
    ctx.addEventListener(document, 'visibilitychange', () => {
      if (document.visibilityState === 'hidden') buffer.flushNow()
    })

    let recorderActive = false
    let stopRecording: (() => void) | null = null
    // WXT invalidates the previous instance when the updated script is injected.
    // Release rrweb observers so existing tabs recover
    // without navigation or a second recorder living alongside the old one.
    const stopRecorder = () => {
      recorderActive = false
      try {
        stopRecording?.()
      } catch (error) {
        console.warn('[browseros-claw replay] stop failed', error)
      } finally {
        stopRecording = null
        buffer.close()
      }
    }
    const onMessage = (
      message: unknown,
      _sender: chrome.runtime.MessageSender,
      sendResponse: (response: unknown) => void,
    ) => {
      const recorderMessage = message as { type?: unknown }
      if (recorderMessage.type === 'recorder-resnapshot') {
        if (recorderActive) {
          try {
            rrweb.record.takeFullSnapshot()
          } catch (error) {
            console.warn('[browseros-claw replay] resnapshot failed', error)
          }
        }
        return false
      }
      // The session that owned this tab has ended: stop recording so the tab is
      // not recorded through the cleanup grace. Events after release never enter
      // any replay window, so nothing worth keeping is lost.
      if (recorderMessage.type === 'recorder-stop') {
        stopRecorder()
        void Promise.all(pendingPosts).then((results) =>
          sendResponse({ persisted: results.every(Boolean) }),
        )
        return true
      }
      return false
    }
    chrome.runtime.onMessage.addListener(onMessage)
    ctx.onInvalidated(() => {
      stopRecorder()
      chrome.runtime.onMessage.removeListener(onMessage)
    })

    try {
      const stop = rrweb.record({
        maskInputOptions: { password: true },
        sampling: {
          mousemove: false,
          scroll: 250,
          media: 500,
          input: 'last',
        },
        recordCanvas: false,
        emit: buffer.emit,
      })
      stopRecording = typeof stop === 'function' ? stop : null
      recorderActive = stopRecording !== null
    } catch (error) {
      console.warn('[browseros-claw replay] rrweb.record threw', error)
      buffer.close()
    }
  },
})
