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
import { BROWSER_TOOLS } from '@browseros/browser-mcp/registry'
import {
  executeTool,
  type ToolDefinition,
} from '@browseros/browser-mcp/tools/framework'
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
  const screenshotTool = BROWSER_TOOLS.find((t) => t.name === 'screenshot')
  if (!screenshotTool) {
    // The browser-mcp catalogue not exposing a screenshot tool would
    // be a contract break we discover at boot; rather than throw and
    // crash the cockpit, log loudly and return a no-op handle so the
    // homepage gracefully falls back to placeholders.
    logger.error(
      'screencast: screenshot tool missing from BROWSER_TOOLS; poller will not start',
    )
    return { stop: () => undefined }
  }

  let running = false
  const tick = async (): Promise<void> => {
    if (running) return
    running = true
    try {
      await runTick(opts.session, screenshotTool, registry)
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
  screenshotTool: ToolDefinition,
  registry: TabActivityRegistry,
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
      batch.map((pageId) => snapOne(pageId, session, screenshotTool)),
    )
  }
}

async function snapOne(
  pageId: number,
  session: BrowserSession,
  screenshotTool: ToolDefinition,
): Promise<void> {
  try {
    const result = await executeTool(
      screenshotTool,
      {
        page: pageId,
        format: 'jpeg',
        quality: JPEG_QUALITY,
        annotate: false,
      },
      {
        session,
        signal: AbortSignal.timeout(SCREENSHOT_TIMEOUT_MS),
      },
    )
    if (result.isError) {
      // Drop the cached frame once we cross the backoff threshold.
      // Holding on to the previous JPEG after the agent has navigated
      // away (e.g. into a cross-origin iframe that the screenshot tool
      // cannot capture) means /tabs/activity returns the OLD page's
      // image with the NEW page's URL + title until backoff lifts.
      // One transient failure still keeps the frame (cheap recovery
      // for one-off CDP hiccups); sustained failures drop it so the
      // UI falls back to the placeholder honestly.
      if (screencastCache.markFailure(pageId)) {
        screencastCache.clearFrame(pageId)
      }
      return
    }
    const image = extractImage(result.structuredContent)
    if (!image) {
      if (screencastCache.markFailure(pageId)) {
        screencastCache.clearFrame(pageId)
      }
      return
    }
    screencastCache.set(pageId, {
      jpegBase64: image,
      capturedAt: Date.now(),
      byteLength: estimateBase64Bytes(image),
    })
    screencastCache.clearFailure(pageId)
  } catch (err) {
    logger.warn('screencast: snap failed', {
      pageId,
      error: err instanceof Error ? err.message : String(err),
    })
    if (screencastCache.markFailure(pageId)) {
      screencastCache.delete(pageId)
    }
  }
}

function extractImage(structured: unknown): string | null {
  if (!structured || typeof structured !== 'object') return null
  const sc = structured as Record<string, unknown>
  const image = sc.image
  if (typeof image !== 'string' || image.length === 0) return null
  return image
}

function estimateBase64Bytes(b64: string): number {
  // Standard base64 expansion: 4 chars per 3 bytes, minus 1 byte per
  // trailing '=' padding.
  const padding = b64.endsWith('==') ? 2 : b64.endsWith('=') ? 1 : 0
  return (b64.length * 3) / 4 - padding
}
