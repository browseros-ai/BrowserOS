import { describe, expect, it } from 'bun:test'
import type {
  ScreenshotCaptureOptions,
  ScreenshotCaptureResult,
} from '../../../src/browser/core/screenshot'
import type { BrowserSession } from '../../../src/browser/core/session'
import { executeTool } from '../../../src/tools/browser/framework'
import { screenshot } from '../../../src/tools/browser/screenshot'

describe('screenshot tool', () => {
  it('defaults annotate to true and returns inline JPEG content', async () => {
    let captured:
      | { page: number; options: ScreenshotCaptureOptions }
      | undefined
    const session = {
      screenshot: async (page: number, options: ScreenshotCaptureOptions) => {
        captured = { page, options }
        return {
          data: 'jpeg-data',
          mimeType: `image/${options.format}`,
          annotations: [
            {
              ref: 'e1',
              number: 1,
              role: 'button',
              name: 'Save',
              box: { x: 1, y: 2, width: 3, height: 4 },
            },
          ],
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
        annotate: true,
        clip: { x: 5, y: 7, width: 2048, height: 1536, scale: 0.5 },
      },
    })
    expect(result.content).toEqual([
      { type: 'image', data: 'jpeg-data', mimeType: 'image/jpeg' },
    ])
    expect(result.structuredContent).toEqual({
      page: 3,
      annotations: [
        {
          ref: 'e1',
          number: 1,
          role: 'button',
          name: 'Save',
          box: { x: 1, y: 2, width: 3, height: 4 },
        },
      ],
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
      { page: 3, fullPage: true, annotate: false },
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
    expect(result.structuredContent).toBeUndefined()
  })
})
