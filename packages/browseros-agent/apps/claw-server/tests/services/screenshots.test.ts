/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Covers both branches of persistScreenshot AND the first-capture
 * policy that guarantees at least one visual anchor per tab.
 *
 *   1. Tool-result branch: image bytes in the tool result get written
 *      regardless of tool name.
 *   2. Screencast-fallback branch: state-mutating page-targeted
 *      dispatches with no image bytes AND a cache frame for the
 *      pageId AND the env flag on -> cache bytes get written.
 *   3. First-capture override: the FIRST read-only dispatch on a
 *      given (agentId, pageId) pair also writes so an all-read-only
 *      audit still has a visual anchor for each tab. Subsequent
 *      read-only dispatches on the same tab skip.
 * Plus the guard rails: read-only deny-list-after-first, null pageId,
 * empty cache, env flag off, isError true, cross-agent isolation.
 */

import { afterEach, beforeEach, describe, expect, it } from 'bun:test'
import { existsSync, readFileSync } from 'node:fs'
import { env } from '../../src/env'
import { agentTabs } from '../../src/lib/agent-tabs'
import { screencastCache } from '../../src/services/screencast-cache'
import {
  persistScreenshot,
  screenshotPath,
} from '../../src/services/screenshots'
import { withTempBrowserClawDir } from '../_helpers/temp-browserclaw-dir'

const ONE_PX_JPEG_B64 =
  '/9j/4AAQSkZJRgABAQEASABIAAD/2wBDAAMCAgICAgMCAgIDAwMDBAYEBAQEBAgGBgUGCQgKCgkICQkKDA8MCgsOCwkJDRENDg8QEBEQCgwSExIQEw8QEBD/2wBDAQMDAwQDBAgEBAgQCwkLEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBD/wAARCAABAAEDASIAAhEBAxEB/8QAFQABAQAAAAAAAAAAAAAAAAAAAAr/xAAUEAEAAAAAAAAAAAAAAAAAAAAA/8QAFAEBAAAAAAAAAAAAAAAAAAAAAP/EABQRAQAAAAAAAAAAAAAAAAAAAAD/2gAMAwEAAhEDEQA/AKpAA//Z'

const AGENT = 'test-agent'

function primeCache(pageId: number, b64: string = ONE_PX_JPEG_B64): void {
  const raw = Buffer.from(b64, 'base64')
  screencastCache.set(pageId, {
    jpegBase64: b64,
    capturedAt: 1_000_000,
    byteLength: raw.length,
  })
}

const ORIGINAL_FALLBACK = env.screencastScreenshotFallback

describe('persistScreenshot', () => {
  beforeEach(() => {
    screencastCache.resetForTesting()
    agentTabs.clear()
    env.screencastScreenshotFallback = true
  })
  afterEach(() => {
    screencastCache.resetForTesting()
    agentTabs.clear()
    env.screencastScreenshotFallback = ORIGINAL_FALLBACK
  })

  it('writes <dispatchId>.jpg from tool-result image content (explicit screenshot tool)', async () => {
    await withTempBrowserClawDir(async () => {
      persistScreenshot({
        dispatchId: 42,
        toolName: 'screenshot',
        pageId: 1,
        agentId: AGENT,
        result: {
          isError: false,
          content: [
            { type: 'image', data: ONE_PX_JPEG_B64, mimeType: 'image/jpeg' },
          ],
          structuredContent: { page: 1, format: 'jpeg', bytes: 0 },
        },
      })
      await new Promise((r) => setTimeout(r, 50))
      const path = screenshotPath(42)
      expect(existsSync(path)).toBe(true)
      expect(readFileSync(path).length).toBeGreaterThan(0)
    })
  })

  it('legacy structured.image field still routes through the tool-result branch', async () => {
    await withTempBrowserClawDir(async () => {
      persistScreenshot({
        dispatchId: 4,
        toolName: 'screenshot',
        pageId: 1,
        agentId: AGENT,
        result: {
          isError: false,
          content: [],
          structuredContent: { image: ONE_PX_JPEG_B64 },
        },
      })
      await new Promise((r) => setTimeout(r, 50))
      expect(existsSync(screenshotPath(4))).toBe(true)
    })
  })

  it('no-op when isError=true even if tool-result carries image bytes AND cache has a frame', async () => {
    await withTempBrowserClawDir(async () => {
      primeCache(1)
      persistScreenshot({
        dispatchId: 2,
        toolName: 'screenshot',
        pageId: 1,
        agentId: AGENT,
        result: {
          isError: true,
          content: [
            { type: 'image', data: ONE_PX_JPEG_B64, mimeType: 'image/jpeg' },
          ],
          structuredContent: {},
        },
      })
      await new Promise((r) => setTimeout(r, 30))
      expect(existsSync(screenshotPath(2))).toBe(false)
    })
  })

  it('screencast fallback: state-mutating dispatch with no image bytes + cache frame writes cache bytes', async () => {
    await withTempBrowserClawDir(async () => {
      primeCache(7)
      persistScreenshot({
        dispatchId: 100,
        toolName: 'navigate',
        pageId: 7,
        agentId: AGENT,
        result: {
          isError: false,
          content: [{ type: 'text', text: 'navigated' }],
          structuredContent: { ok: true },
        },
      })
      await new Promise((r) => setTimeout(r, 50))
      const path = screenshotPath(100)
      expect(existsSync(path)).toBe(true)
      expect(readFileSync(path).length).toBeGreaterThan(0)
    })
  })

  it('screencast fallback: `act`, `tabs`, `evaluate` (state-mutating) also get cache bytes', async () => {
    await withTempBrowserClawDir(async () => {
      primeCache(3)
      for (const [dispatchId, toolName] of [
        [201, 'act'],
        [202, 'tabs'],
        [203, 'evaluate'],
      ] as const) {
        persistScreenshot({
          dispatchId,
          toolName,
          pageId: 3,
          agentId: AGENT,
          result: {
            isError: false,
            content: [{ type: 'text', text: 'ok' }],
            structuredContent: {},
          },
        })
      }
      await new Promise((r) => setTimeout(r, 50))
      expect(existsSync(screenshotPath(201))).toBe(true)
      expect(existsSync(screenshotPath(202))).toBe(true)
      expect(existsSync(screenshotPath(203))).toBe(true)
    })
  })

  it('read-only tools SKIP the fallback once the page has been captured before', async () => {
    // Simulates the "already got a visual anchor for this tab" case:
    // pre-mark first-capture-done, then verify snapshot / read / grep
    // / diff / wait do NOT write.
    await withTempBrowserClawDir(async () => {
      primeCache(9)
      agentTabs.markFirstCaptureDone(AGENT, 9)
      for (const [dispatchId, toolName] of [
        [301, 'snapshot'],
        [302, 'read'],
        [303, 'grep'],
        [304, 'diff'],
        [305, 'wait'],
      ] as const) {
        persistScreenshot({
          dispatchId,
          toolName,
          pageId: 9,
          agentId: AGENT,
          result: {
            isError: false,
            content: [{ type: 'text', text: 'read result' }],
            structuredContent: {},
          },
        })
      }
      await new Promise((r) => setTimeout(r, 50))
      for (const dispatchId of [301, 302, 303, 304, 305]) {
        expect(existsSync(screenshotPath(dispatchId))).toBe(false)
      }
    })
  })

  it('screencast fallback SKIPS when pageId is null', async () => {
    await withTempBrowserClawDir(async () => {
      persistScreenshot({
        dispatchId: 400,
        toolName: 'navigate',
        pageId: null,
        agentId: AGENT,
        result: {
          isError: false,
          content: [{ type: 'text', text: 'navigated' }],
          structuredContent: {},
        },
      })
      await new Promise((r) => setTimeout(r, 30))
      expect(existsSync(screenshotPath(400))).toBe(false)
    })
  })

  it('screencast fallback SKIPS when the cache has no frame for the pageId', async () => {
    await withTempBrowserClawDir(async () => {
      // Cache primed for a DIFFERENT pageId only.
      primeCache(50)
      persistScreenshot({
        dispatchId: 500,
        toolName: 'navigate',
        pageId: 51,
        agentId: AGENT,
        result: {
          isError: false,
          content: [{ type: 'text', text: 'navigated' }],
          structuredContent: {},
        },
      })
      await new Promise((r) => setTimeout(r, 30))
      expect(existsSync(screenshotPath(500))).toBe(false)
    })
  })

  it('screencast fallback SKIPS when env.screencastScreenshotFallback is off (tool-result branch still fires)', async () => {
    await withTempBrowserClawDir(async () => {
      env.screencastScreenshotFallback = false
      primeCache(11)
      persistScreenshot({
        dispatchId: 600,
        toolName: 'navigate',
        pageId: 11,
        agentId: AGENT,
        result: {
          isError: false,
          content: [{ type: 'text', text: 'navigated' }],
          structuredContent: {},
        },
      })
      await new Promise((r) => setTimeout(r, 30))
      expect(existsSync(screenshotPath(600))).toBe(false)
      // Sanity: tool-result branch STILL fires when flag is off.
      persistScreenshot({
        dispatchId: 601,
        toolName: 'screenshot',
        pageId: 11,
        agentId: AGENT,
        result: {
          isError: false,
          content: [
            { type: 'image', data: ONE_PX_JPEG_B64, mimeType: 'image/jpeg' },
          ],
          structuredContent: {},
        },
      })
      await new Promise((r) => setTimeout(r, 50))
      expect(existsSync(screenshotPath(601))).toBe(true)
    })
  })

  it('tool-result branch wins over cache when both are available', async () => {
    await withTempBrowserClawDir(async () => {
      const CACHE_B64 = ONE_PX_JPEG_B64
      const TOOL_B64 =
        '/9j/4AAQSkZJRgABAAEASABIAAD/2wBDAAgGBgcGBQgHBwcJCQgKDBQNDAsLDBkSEw8UHRofHh0aHBwgJC4nICIsIxwcKDcpLDAxNDQ0Hyc5PTgyPC4zNDL/wAALCAABAAEBAREA/8QAFAABAAAAAAAAAAAAAAAAAAAACv/EABQBAQAAAAAAAAAAAAAAAAAAAAD/2gAIAQEAAD8Aqp//2Q=='
      primeCache(1, CACHE_B64)
      persistScreenshot({
        dispatchId: 700,
        toolName: 'screenshot',
        pageId: 1,
        agentId: AGENT,
        result: {
          isError: false,
          content: [{ type: 'image', data: TOOL_B64, mimeType: 'image/jpeg' }],
          structuredContent: {},
        },
      })
      await new Promise((r) => setTimeout(r, 50))
      const written = readFileSync(screenshotPath(700))
      expect(written).toEqual(Buffer.from(TOOL_B64, 'base64'))
      expect(written).not.toEqual(Buffer.from(CACHE_B64, 'base64'))
    })
  })

  it('FIRST-CAPTURE: first read on a page writes even though `read` is in the deny-list', async () => {
    await withTempBrowserClawDir(async () => {
      primeCache(20)
      persistScreenshot({
        dispatchId: 800,
        toolName: 'read',
        pageId: 20,
        agentId: AGENT,
        result: {
          isError: false,
          content: [{ type: 'text', text: 'page content' }],
          structuredContent: {},
        },
      })
      await new Promise((r) => setTimeout(r, 50))
      expect(existsSync(screenshotPath(800))).toBe(true)
      expect(agentTabs.hasFirstCapture(AGENT, 20)).toBe(true)
    })
  })

  it('FIRST-CAPTURE: second read on the SAME page does NOT write', async () => {
    await withTempBrowserClawDir(async () => {
      primeCache(21)
      // First read fires the override + marks first-capture-done.
      persistScreenshot({
        dispatchId: 900,
        toolName: 'read',
        pageId: 21,
        agentId: AGENT,
        result: {
          isError: false,
          content: [{ type: 'text', text: 'first read' }],
          structuredContent: {},
        },
      })
      // Second read on the same page hits the deny-list and skips.
      persistScreenshot({
        dispatchId: 901,
        toolName: 'read',
        pageId: 21,
        agentId: AGENT,
        result: {
          isError: false,
          content: [{ type: 'text', text: 'second read' }],
          structuredContent: {},
        },
      })
      await new Promise((r) => setTimeout(r, 50))
      expect(existsSync(screenshotPath(900))).toBe(true)
      expect(existsSync(screenshotPath(901))).toBe(false)
    })
  })

  it('FIRST-CAPTURE: state-mutating write also marks; subsequent read on same page skips', async () => {
    await withTempBrowserClawDir(async () => {
      primeCache(22)
      persistScreenshot({
        dispatchId: 1000,
        toolName: 'navigate',
        pageId: 22,
        agentId: AGENT,
        result: {
          isError: false,
          content: [{ type: 'text', text: 'navigated' }],
          structuredContent: {},
        },
      })
      // Read on the same page should now skip because the navigate
      // already marked this page as first-captured.
      persistScreenshot({
        dispatchId: 1001,
        toolName: 'read',
        pageId: 22,
        agentId: AGENT,
        result: {
          isError: false,
          content: [{ type: 'text', text: 'read after navigate' }],
          structuredContent: {},
        },
      })
      await new Promise((r) => setTimeout(r, 50))
      expect(existsSync(screenshotPath(1000))).toBe(true)
      expect(existsSync(screenshotPath(1001))).toBe(false)
    })
  })

  it('FIRST-CAPTURE: two agents on the same pageId EACH get their own first-capture write', async () => {
    await withTempBrowserClawDir(async () => {
      primeCache(30)
      persistScreenshot({
        dispatchId: 1100,
        toolName: 'read',
        pageId: 30,
        agentId: 'agent-a',
        result: {
          isError: false,
          content: [{ type: 'text', text: 'a reads' }],
          structuredContent: {},
        },
      })
      persistScreenshot({
        dispatchId: 1101,
        toolName: 'read',
        pageId: 30,
        agentId: 'agent-b',
        result: {
          isError: false,
          content: [{ type: 'text', text: 'b reads' }],
          structuredContent: {},
        },
      })
      await new Promise((r) => setTimeout(r, 50))
      expect(existsSync(screenshotPath(1100))).toBe(true)
      expect(existsSync(screenshotPath(1101))).toBe(true)
      expect(agentTabs.hasFirstCapture('agent-a', 30)).toBe(true)
      expect(agentTabs.hasFirstCapture('agent-b', 30)).toBe(true)
    })
  })

  it('FIRST-CAPTURE: cannot override when agentId is null (falls back to strict deny-list)', async () => {
    await withTempBrowserClawDir(async () => {
      primeCache(40)
      persistScreenshot({
        dispatchId: 1200,
        toolName: 'read',
        pageId: 40,
        agentId: null, // e.g. identity resolution failed
        result: {
          isError: false,
          content: [{ type: 'text', text: 'read' }],
          structuredContent: {},
        },
      })
      await new Promise((r) => setTimeout(r, 30))
      expect(existsSync(screenshotPath(1200))).toBe(false)
    })
  })
})
