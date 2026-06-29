/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Recorder content script. Injected by the background service
 * worker (entrypoints/background.ts) via
 * chrome.scripting.executeScript({files: ['recorder.content.js']})
 * into ONLY the chrome tabs the cockpit reports as agent-driven.
 * Operator-owned tabs never load this script because the manifest
 * declares no `content_scripts` block.
 *
 * Lifecycle:
 *
 *   1. Background injects this script after resolving the chrome
 *      tab id for a /replay/tabs row.
 *   2. Script sends `recorder-hello` to the background.
 *   3. Background replies with `recorder-config { sessionId,
 *      tabPageId }` (or `recorder-not-yet` if the map is briefly
 *      out of sync; retry after 1s).
 *   4. Script calls rrweb.record with the throttled config.
 *   5. Events buffer; flush every 2.5s OR every 50 events:
 *      POST to http://127.0.0.1:9200/audit/replay/<sid>/events.
 *   6. On `pagehide`, navigator.sendBeacon flush so unload events
 *      are not dropped.
 *   7. On `recorder-stop` message: final flush, rrweb.stop(),
 *      the script just stops emitting.
 *
 * Throttling (carried over from F2 in the recorder-stability
 * tracker): sampling.mousemove off, scroll 250ms, input 'last',
 * recordCanvas false, maskInputOptions password true. JSON
 * serialisation happens off the rrweb hot path via queueMicrotask.
 *
 * window.__browserosClawReplayInstalled is a re-injection guard
 * so a background poll that races a chrome.scripting injection
 * does not double-install.
 */

import * as rrweb from 'rrweb'
import { defineContentScript } from 'wxt/utils/define-content-script'
import type { RecorderMessage } from '@/modules/replay-background'

const COCKPIT_ORIGIN = 'http://127.0.0.1:9200'
const BUFFER_CAP = 500
const FLUSH_INTERVAL_MS = 2_500
const FLUSH_AT_SIZE = 50

export default defineContentScript({
  matches: [],
  registration: 'runtime', // declares the script can be injected via chrome.scripting
  main() {
    // Guard against double-install when the background's
    // chrome.scripting.executeScript runs against a tab whose
    // prior content script context has not yet been torn down by
    // a navigation. The flag is per-document; navigation creates
    // a new context where this flag is unset.
    type Marked = typeof window & { __browserosClawReplayInstalled?: boolean }
    if ((window as Marked).__browserosClawReplayInstalled) return
    ;(window as Marked).__browserosClawReplayInstalled = true

    void run()
  },
})

async function run(): Promise<void> {
  const config = await fetchConfig()
  if (!config) return

  const eventsUrl =
    `${COCKPIT_ORIGIN}/audit/replay/${config.sessionId}/events` as const
  const tabPageId = config.tabPageId

  const buf: string[] = []
  let dropped = 0
  let timer: ReturnType<typeof setTimeout> | null = null
  const rawQueue: unknown[] = []
  let pendingSerialise: 1 | null = null
  let stopper: (() => void) | undefined

  function send(body: string): void {
    try {
      if (
        typeof navigator.sendBeacon === 'function' &&
        document.visibilityState === 'hidden'
      ) {
        navigator.sendBeacon(
          eventsUrl,
          new Blob([body], { type: 'application/x-ndjson' }),
        )
        return
      }
      void fetch(eventsUrl, {
        method: 'POST',
        headers: { 'content-type': 'application/x-ndjson' },
        body,
        credentials: 'omit',
      }).catch((err) => {
        // eslint-disable-next-line no-console
        console.warn('[browseros-claw replay] events fetch failed', err)
      })
    } catch (err) {
      // eslint-disable-next-line no-console
      console.warn('[browseros-claw replay] send threw', err)
    }
  }

  function flush(): void {
    if (buf.length === 0) return
    const body = buf.join('\n')
    buf.length = 0
    if (dropped > 0) {
      // eslint-disable-next-line no-console
      console.warn(
        '[browseros-claw replay] dropped',
        dropped,
        'events under buffer pressure',
      )
      dropped = 0
    }
    send(body)
  }

  function armFlushTimer(): void {
    if (timer !== null) return
    timer = setTimeout(() => {
      timer = null
      flush()
    }, FLUSH_INTERVAL_MS)
  }

  function drainRawQueue(): void {
    pendingSerialise = null
    for (const event of rawQueue) {
      let line: string
      try {
        // biome-ignore lint/suspicious/noExplicitAny: rrweb's event
        // union is wide; we trust the recorder's output shape.
        const ev = event as { timestamp?: number; type?: number; data?: any }
        line = JSON.stringify({
          tabPageId,
          ts: typeof ev.timestamp === 'number' ? ev.timestamp : Date.now(),
          type: ev.type,
          data: ev.data,
        })
      } catch {
        continue
      }
      if (buf.length >= BUFFER_CAP) {
        buf.shift()
        dropped++
      }
      buf.push(line)
      if (buf.length >= FLUSH_AT_SIZE) flush()
    }
    rawQueue.length = 0
    armFlushTimer()
  }

  try {
    stopper = rrweb.record({
      maskInputOptions: { password: true },
      sampling: {
        mousemove: false,
        scroll: 250,
        media: 500,
        input: 'last',
      },
      recordCanvas: false,
      emit(event) {
        rawQueue.push(event)
        if (pendingSerialise === null) {
          pendingSerialise = 1
          if (typeof queueMicrotask === 'function') {
            queueMicrotask(drainRawQueue)
          } else {
            setTimeout(drainRawQueue, 0)
          }
        }
      },
    })
    // eslint-disable-next-line no-console
    console.info('[browseros-claw replay] recorder online', {
      sessionId: config.sessionId,
      tabPageId,
    })
  } catch (err) {
    // eslint-disable-next-line no-console
    console.warn('[browseros-claw replay] rrweb.record threw', err)
    return
  }

  function flushNow(): void {
    if (rawQueue.length > 0) drainRawQueue()
    flush()
  }

  function stop(): void {
    flushNow()
    try {
      stopper?.()
    } catch {
      // ignore; rrweb may already be torn down
    }
  }

  window.addEventListener('pagehide', flushNow)
  window.addEventListener('visibilitychange', () => {
    if (document.visibilityState === 'hidden') flushNow()
  })

  chrome.runtime.onMessage.addListener((message) => {
    const msg = message as RecorderMessage
    if (msg.type === 'recorder-stop') stop()
    return false
  })
}

/**
 * Asks the background worker for this tab's recorder config. The
 * background may briefly reply 'recorder-not-yet' if the cockpit
 * poll has not yet resolved this chrome tab id; retry once after
 * 1s. After two not-yet responses we give up; the next /replay/tabs
 * poll will trigger another injection.
 */
async function fetchConfig(): Promise<{
  sessionId: string
  tabPageId: number
} | null> {
  for (let attempt = 0; attempt < 2; attempt++) {
    if (attempt > 0) {
      await new Promise((r) => setTimeout(r, 1_000))
    }
    let response: RecorderMessage | undefined
    try {
      response = (await chrome.runtime.sendMessage({
        type: 'recorder-hello',
      } satisfies RecorderMessage)) as RecorderMessage | undefined
    } catch {
      return null
    }
    if (!response) return null
    if (response.type === 'recorder-config') {
      return {
        sessionId: response.sessionId,
        tabPageId: response.tabPageId,
      }
    }
  }
  return null
}
