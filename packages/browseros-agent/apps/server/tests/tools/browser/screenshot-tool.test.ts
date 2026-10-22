import { describe, expect, it } from 'bun:test'
import type {
  ScreenshotCaptureOptions,
  ScreenshotCaptureResult,
} from '@browseros/browser-core/core/screenshot'
import type { BrowserSession } from '@browseros/browser-core/core/session'
import { executeTool } from '@browseros/browser-mcp/tools/framework'
import { screenshot } from '@browseros/browser-mcp/tools/screenshot'

describe('screenshot tool', () => {
  it('defaults annotate to false and returns inline JPEG content', async () => {
    let captured:
      | { page: number; options: ScreenshotCaptureOptions }
      | undefined
    const session = {
      screenshot: async (page: number, options: ScreenshotCaptureOptions) => {
        captured = { page, options }
        return {
          data: 'jpeg-data',
          mimeType: `image/${options.format}`,
          annotations: options.annotate
            ? [
                {
                  ref: 'e1',
                  number: 1,
                  role: 'button',
                  name: 'Save',
                  box: { x: 1, y: 2, width: 3, height: 4 },
                },
              ]
            : [],
        } satisfies ScreenshotCaptureResult
      },
      pages: {
        getSession: async () => ({
          session: {
            Page: {
              getLayoutMetrics: async () => ({
                layoutViewport: {
                  pageX: 0,
                  pageY: 0,
                  clientWidth: 2048,
                  clientHeight: 1536,
                },
                cssLayoutViewport: {
                  pageX: 5,
                  pageY: 7,
                  clientWidth: 2048,
                  clientHeight: 1536,
                },
              }),
            },
          },
        }),
      },
    } as unknown as BrowserSession

    const result = await executeTool(screenshot, { page: 3 }, { session })

    expect(captured).toEqual({
      page: 3,
      options: {
        format: 'jpeg',
        quality: 80,
        fullPage: false,
        annotate: false,
        clip: { x: 5, y: 7, width: 2048, height: 1536, scale: 1 },
      },
    })
    expect(result.content).toEqual([
      { type: 'image', data: 'jpeg-data', mimeType: 'image/jpeg' },
    ])
    expect(result.structuredContent).toEqual({
      page: 3,
      format: 'jpeg',
      bytes: Buffer.from('jpeg-data', 'base64').length,
    })
  })

  it('passes annotate false through to the browser session', async () => {
    let captured:
      | { page: number; options: ScreenshotCaptureOptions }
      | undefined
    const session = {
      screenshot: async (page: number, options: ScreenshotCaptureOptions) => {
        captured = { page, options }
        return {
          data: 'jpeg-data',
          mimeType: `image/${options.format}`,
          annotations: [],
        } satisfies ScreenshotCaptureResult
      },
      pages: {},
    } as unknown as BrowserSession

    const result = await executeTool(
      screenshot,
      { page: 3, fullPage: true, annotate: false, size: { width: 1 } },
      { session },
    )

    expect(captured).toEqual({
      page: 3,
      options: {
        format: 'jpeg',
        quality: 80,
        fullPage: true,
        annotate: false,
      },
    })
    expect(result.content).toEqual([
      { type: 'image', data: 'jpeg-data', mimeType: 'image/jpeg' },
    ])
    expect(result.structuredContent).toEqual({
      page: 3,
      format: 'jpeg',
      bytes: Buffer.from('jpeg-data', 'base64').length,
    })
  })

  it.each([
    [{ width: 1024, height: 768 }, 768 / 1800],
    [{ width: 1024 }, 1024 / 1440],
    [{ height: 768 }, 768 / 1800],
    [{ width: 4096, height: 4096 }, 1],
    [{}, 1],
  ])('caps only supplied dimensions for size %j', async (size, scale) => {
    let captured: ScreenshotCaptureOptions | undefined
    const session = {
      screenshot: async (_page: number, options: ScreenshotCaptureOptions) => {
        captured = options
        return { data: 'jpeg-data', mimeType: 'image/jpeg', annotations: [] }
      },
      pages: {
        getSession: async () => ({
          session: {
            Page: {
              getLayoutMetrics: async () => ({
                layoutViewport: {
                  pageX: 10,
                  pageY: 14,
                  clientWidth: 2880,
                  clientHeight: 3600,
                },
                cssLayoutViewport: {
                  pageX: 5,
                  pageY: 7,
                  clientWidth: 1440,
                  clientHeight: 1800,
                },
              }),
            },
          },
        }),
      },
    } as unknown as BrowserSession
    await executeTool(screenshot, { page: 3, size }, { session })
    expect(captured?.clip).toEqual({
      x: 5,
      y: 7,
      width: 1440,
      height: 1800,
      scale,
    })
  })

  it.each([{ width: 0 }, { height: 4097 }, { width: 1.5 }])(
    'rejects invalid supplied dimensions %j',
    (size) => {
      expect(screenshot.input.safeParse({ page: 3, size }).success).toBe(false)
    },
  )
})
