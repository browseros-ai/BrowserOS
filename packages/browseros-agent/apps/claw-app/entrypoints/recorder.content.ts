/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import * as rrweb from 'rrweb'
import { defineContentScript } from 'wxt/utils/define-content-script'
import {
  createRecorderBuffer,
  installRecorderFlushListeners,
  type RecorderMessage,
} from '@/modules/recorder'

/** Records each eligible main-frame document from load and relays rrweb batches. */
export default defineContentScript({
  matches: ['<all_urls>'],
  runAt: 'document_start',
  allFrames: false,
  main() {
    type Marked = typeof window & { __browserosClawReplayInstalled?: boolean }
    if ((window as Marked).__browserosClawReplayInstalled) return
    ;(window as Marked).__browserosClawReplayInstalled = true

    const buffer = createRecorderBuffer({
      send(ndjson, hasGap) {
        try {
          void chrome.runtime
            .sendMessage({
              type: 'recorder-events',
              ndjson,
              hasGap,
            } satisfies RecorderMessage)
            .catch((error) => {
              console.warn(
                '[browseros-claw replay] sendMessage to background failed',
                error,
              )
            })
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

    installRecorderFlushListeners({
      page: window,
      document,
      flush: buffer.flushNow,
    })

    let recorderActive = false
    chrome.runtime.onMessage.addListener((message) => {
      const recorderMessage = message as { type?: unknown }
      if (recorderMessage.type !== 'recorder-resnapshot' || !recorderActive) {
        return false
      }
      try {
        rrweb.record.takeFullSnapshot()
      } catch (error) {
        console.warn('[browseros-claw replay] resnapshot failed', error)
      }
      return false
    })

    try {
      const stopRecording = rrweb.record({
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
      recorderActive = typeof stopRecording === 'function'
    } catch (error) {
      console.warn('[browseros-claw replay] rrweb.record threw', error)
      buffer.close()
    }
  },
})
