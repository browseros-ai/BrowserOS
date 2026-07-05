/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Background poller that drives the Running-now homepage screencast.
 * Every `intervalMs` (default 1500) it walks the tab-activity
 * registry, calls the `screenshot` tool against each active tab via
 * `executeTool` (bypassing the agent-MCP register hook so frames
 * never land in the audit log), and writes the result into the
 * screencast cache.
 *
 * Three guard rails keep the poller cheap and resilient:
 *
 *   1. A `running` flag prevents tick overlap. If a tick is still in
 *      flight when the next interval fires, we skip.
 *   2. Per-pageId failure backoff (see screencast-cache.ts). Three
 *      consecutive failures park a page until the registry's
 *      `lastToolAt` advances past the recorded `lastFailureAt`.
 *   3. A bounded concurrency window (MAX_PARALLEL_SHOTS) caps how
 *      many CDP screenshot calls fly at once, so a 20-tab burst does
 *      not stampede the underlying Chromium.
 *
 * Pages whose pageId is no longer in the registry snapshot (the tab
 * closed) get GC'd from the cache at the end of each tick.
 */

import type { BrowserSession } from '@browseros/browser-core/core/session'
import { logger } from '../lib/logger'
import {
  tabActivityRegistry as defaultRegistry,
  type TabActivityRegistry,
} from '../lib/tab-activity'
import { screencastCache } from './screencast-cache'

export const DEFAULT_POLL_INTERVAL_MS = 1500
export const SCREENSHOT_TIMEOUT_MS = 2000
export const MAX_PARALLEL_SHOTS = 8
export const JPEG_QUALITY = 50

export interface ScreencastPollerHandle {
  stop(): void
}

export interface StartScreencastPollerOpts {
  session: BrowserSession
  intervalMs?: number
  /**
   * Injectable registry for tests. Production uses the module-level
   * singleton. Bun's `mock.module` has run-level scope and leaks
   * across files in the same `bun test` run, so an explicit
   * dependency seam is cleaner than mocking the registry import.
   */
  registry?: TabActivityRegistry
}

export function startScreencastPoller(
  opts: StartScreencastPollerOpts,
): ScreencastPollerHandle {
  const intervalMs = opts.intervalMs ?? DEFAULT_POLL_INTERVAL_MS
  const registry = opts.registry ?? defaultRegistry

  // Tracks CDP screenshot calls still resolving on Chromium's side.
  // The `running` flag below stops a tick from overlapping with
  // itself, but it does NOT stop individual snapOne calls from
  // stacking: session.screenshot() has no AbortSignal support, so
  // when a snapOne times out its outer Promise.race settles but the
  // underlying capture keeps running. Under sustained sluggishness
  // each 1.5s tick would fire a fresh CDP call for the same page on
  // top of the last one. inFlight is a per-page guard; snapOne
  // early-returns when the page id is already present, and the
  // guard entry is cleared by the outstanding capture promise's
  // .finally() when the CDP call actually resolves or rejects.
  const inFlight = new Set<number>()

  let running = false
  const tick = async (): Promise<void> => {
    if (running) return
    running = true
    try {
      await runTick(opts.session, registry, inFlight)
    } catch (err) {
      logger.warn('screencast: tick failed', {
        error: err instanceof Error ? err.message : String(err),
      })
    } finally {
      running = false
    }
  }

  const handle = setInterval(() => {
    void tick()
  }, intervalMs)
  // The poller is a background task; do not keep the event loop alive
  // just for the interval. Bun's setInterval returns a Timeout that
  // supports unref() in the Node-compatible shape.
  if (typeof (handle as { unref?: () => void }).unref === 'function') {
    ;(handle as { unref: () => void }).unref()
  }
  // Fire one tick immediately so the first frames land within the
  // poll interval rather than after the first 1500ms delay.
  void tick()

  return {
    stop: () => {
      clearInterval(handle)
    },
  }
}

async function runTick(
  session: BrowserSession,
  registry: TabActivityRegistry,
  inFlight: Set<number>,
): Promise<void> {
  const tabs = registry.snapshot()
  const livePageIds = new Set<number>()

  // 1. Build the work list: active tabs not in failure backoff.
  const work: number[] = []
  for (const tab of tabs) {
    livePageIds.add(tab.pageId)
    if (tab.status !== 'active') continue
    if (screencastCache.isInBackoff(tab.pageId, tab.lastToolAt)) continue
    work.push(tab.pageId)
  }

  // 2. GC frames for tabs that closed since the last tick.
  for (const cachedPageId of screencastCache.snapshot().keys()) {
    if (!livePageIds.has(cachedPageId)) {
      screencastCache.delete(cachedPageId)
    }
  }

  if (work.length === 0) return

  // 3. Bounded-concurrency fan-out: run at most MAX_PARALLEL_SHOTS
  //    snapshots in flight at any time.
  for (let i = 0; i < work.length; i += MAX_PARALLEL_SHOTS) {
    const batch = work.slice(i, i + MAX_PARALLEL_SHOTS)
    await Promise.allSettled(
      batch.map((pageId) => snapOne(pageId, session, inFlight)),
    )
  }
}

async function snapOne(
  pageId: number,
  session: BrowserSession,
  inFlight: Set<number>,
): Promise<void> {
  // Per-page in-flight guard: see the docstring on `inFlight` at
  // startScreencastPoller for why this is not covered by the
  // tick-level `running` flag.
  if (inFlight.has(pageId)) return
  inFlight.add(pageId)

  // Attach the .finally() BEFORE Promise.race so the guard entry is
  // released when the CDP call actually resolves, not when the race
  // timeout wins. If session.screenshot() rejects, .finally() still
  // fires and Promise.race sees the rejection through this chained
  // promise, so no unhandled rejection is orphaned.
  const capturePromise = session
    .screenshot(pageId, {
      format: 'jpeg',
      quality: JPEG_QUALITY,
      annotate: false,
    })
    .finally(() => {
      inFlight.delete(pageId)
    })

  try {
    // Call session.screenshot() directly, WITHOUT a clip. The MCP
    // screenshot tool's default path (via executeTool) computes
    // clip = { width, height, scale = min(1, targetW/vw, targetH/vh) }
    // to fit the capture inside a 1024x768 target. When the actual
    // viewport is bigger than 1024x768 the scale is < 1, and on the
    // BrowserOS Chromium fork Page.captureScreenshot({clip: {scale
    // != 1}}) visibly resizes the tab the user is watching for the
    // duration of the capture, then restores. The poller runs every
    // 1.5s, so the operator sees the driven tab flicker (shrink
    // then expand) at the poll cadence.
    //
    // The poller does not need the tool's size-capping behaviour:
    // MiniScreencast paints frames at a fixed 132px height and
    // downscales in the browser. Capturing at the natural viewport
    // (scale = 1, no clip) keeps the JPEG cost roughly the same
    // after downscaling and avoids the reflow.
    const result = await Promise.race([
      capturePromise,
      new Promise<never>((_, reject) => {
        AbortSignal.timeout(SCREENSHOT_TIMEOUT_MS).addEventListener(
          'abort',
          () => reject(new Error('screenshot timeout')),
        )
      }),
    ])
    if (!result.data || result.data.length === 0) {
      // Drop the cached frame once we cross the backoff threshold.
      // Holding on to the previous JPEG after the agent has navigated
      // away (e.g. into a cross-origin iframe that the screenshot
      // path cannot capture) means /tabs/activity returns the OLD
      // page's image with the NEW page's URL + title until backoff
      // lifts. One transient failure still keeps the frame (cheap
      // recovery for one-off CDP hiccups); sustained failures drop
      // it so the UI falls back to the placeholder honestly.
      if (screencastCache.markFailure(pageId)) {
        screencastCache.clearFrame(pageId)
      }
      return
    }
    screencastCache.set(pageId, {
      jpegBase64: result.data,
      capturedAt: Date.now(),
      byteLength: estimateBase64Bytes(result.data),
    })
    screencastCache.clearFailure(pageId)
  } catch (err) {
    // Match the pre-refactor semantic: on backoff crossing, drop
    // the stale frame but KEEP the failure counter so isInBackoff
    // still reports true until a fresh dispatch on the tab lifts
    // the block. The previous impl called .delete() from this
    // branch, but that only fired when executeTool itself threw;
    // the far more common CDP soft-failure landed in the isError
    // branch which used clearFrame. session.screenshot() has no
    // soft-failure mode - every failure throws - so consolidate on
    // the more forgiving clearFrame path.
    logger.warn('screencast: snap failed', {
      pageId,
      error: err instanceof Error ? err.message : String(err),
    })
    if (screencastCache.markFailure(pageId)) {
      screencastCache.clearFrame(pageId)
    }
  }
}

function estimateBase64Bytes(b64: string): number {
  // Standard base64 expansion: 4 chars per 3 bytes, minus 1 byte per
  // trailing '=' padding.
  const padding = b64.endsWith('==') ? 2 : b64.endsWith('=') ? 1 : 0
  return (b64.length * 3) / 4 - padding
}
