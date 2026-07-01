/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Covers both branches of persistScreenshot:
 *   1. Tool-result branch: image bytes in the tool result get written
 *      regardless of tool name.
 *   2. Screencast-fallback branch: state-mutating page-targeted
 *      dispatches with no image bytes AND a cache frame for the
 *      pageId AND the env flag on -> cache bytes get written.
 * Plus the guard rails: read-only deny-list, null pageId, empty
 * cache, env flag off, isError true.
 */

import { afterEach, beforeEach, describe, expect, it } from 'bun:test'
import { existsSync, readFileSync } from 'node:fs'
import { env } from '../../src/env'
import { screencastCache } from '../../src/services/screencast-cache'
import {
  persistScreenshot,
  screenshotPath,
} from '../../src/services/screenshots'
import { withTempBrowserosDir } from '../_helpers/temp-browseros-dir'

const ONE_PX_JPEG_B64 =
  '/9j/4AAQSkZJRgABAQEASABIAAD/2wBDAAMCAgICAgMCAgIDAwMDBAYEBAQEBAgGBgUGCQgKCgkICQkKDA8MCgsOCwkJDRENDg8QEBEQCgwSExIQEw8QEBD/2wBDAQMDAwQDBAgEBAgQCwkLEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBD/wAARCAABAAEDASIAAhEBAxEB/8QAFQABAQAAAAAAAAAAAAAAAAAAAAr/xAAUEAEAAAAAAAAAAAAAAAAAAAAA/8QAFAEBAAAAAAAAAAAAAAAAAAAAAP/EABQRAQAAAAAAAAAAAAAAAAAAAAD/2gAMAwEAAhEDEQA/AKpAA//Z'

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
    env.screencastScreenshotFallback = true
  })
  afterEach(() => {
    screencastCache.resetForTesting()
    env.screencastScreenshotFallback = ORIGINAL_FALLBACK
  })

  it('writes <dispatchId>.jpg from tool-result image content (explicit screenshot tool)', async () => {
    await withTempBrowserosDir(async () => {
      persistScreenshot({
        dispatchId: 42,
        toolName: 'screenshot',
        pageId: 1,
        result: {
          isError: false,
          content: [
            {
              type: 'image',
              data: ONE_PX_JPEG_B64,
              mimeType: 'image/jpeg',
            },
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
    await withTempBrowserosDir(async () => {
      persistScreenshot({
        dispatchId: 4,
        toolName: 'screenshot',
        pageId: 1,
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
    await withTempBrowserosDir(async () => {
      primeCache(1)
      persistScreenshot({
        dispatchId: 2,
        toolName: 'screenshot',
        pageId: 1,
        result: {
          isError: true,
          content: [
            {
              type: 'image',
              data: ONE_PX_JPEG_B64,
              mimeType: 'image/jpeg',
            },
          ],
          structuredContent: {},
        },
      })
      await new Promise((r) => setTimeout(r, 30))
      expect(existsSync(screenshotPath(2))).toBe(false)
    })
  })

  it('screencast fallback: state-mutating dispatch with no image bytes + cache frame writes cache bytes', async () => {
    await withTempBrowserosDir(async () => {
      primeCache(7)
      persistScreenshot({
        dispatchId: 100,
        toolName: 'navigate',
        pageId: 7,
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
    await withTempBrowserosDir(async () => {
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

  it('screencast fallback SKIPS read-only tools (snapshot / read / grep / diff / wait)', async () => {
    await withTempBrowserosDir(async () => {
      primeCache(9)
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
    await withTempBrowserosDir(async () => {
      persistScreenshot({
        dispatchId: 400,
        toolName: 'navigate',
        pageId: null,
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
    await withTempBrowserosDir(async () => {
      // Cache primed for a DIFFERENT pageId only.
      primeCache(50)
      persistScreenshot({
        dispatchId: 500,
        toolName: 'navigate',
        pageId: 51,
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
    await withTempBrowserosDir(async () => {
      env.screencastScreenshotFallback = false
      primeCache(11)
      persistScreenshot({
        dispatchId: 600,
        toolName: 'navigate',
        pageId: 11,
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
        result: {
          isError: false,
          content: [
            {
              type: 'image',
              data: ONE_PX_JPEG_B64,
              mimeType: 'image/jpeg',
            },
          ],
          structuredContent: {},
        },
      })
      await new Promise((r) => setTimeout(r, 50))
      expect(existsSync(screenshotPath(601))).toBe(true)
    })
  })

  it('tool-result branch wins over cache when both are available', async () => {
    // If the tool result already carried image bytes, we use those
    // instead of the (possibly older) cache frame. Prove it with
    // distinguishable byte payloads.
    await withTempBrowserosDir(async () => {
      const CACHE_B64 = ONE_PX_JPEG_B64
      const TOOL_B64 =
        '/9j/4AAQSkZJRgABAAEASABIAAD/2wBDAAgGBgcGBQgHBwcJCQgKDBQNDAsLDBkSEw8UHRofHh0aHBwgJC4nICIsIxwcKDcpLDAxNDQ0Hyc5PTgyPC4zNDL/wAALCAABAAEBAREA/8QAFAABAAAAAAAAAAAAAAAAAAAACv/EABQBAQAAAAAAAAAAAAAAAAAAAAD/2gAIAQEAAD8Aqp//2Q=='
      primeCache(1, CACHE_B64)
      persistScreenshot({
        dispatchId: 700,
        toolName: 'screenshot',
        pageId: 1,
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
})
