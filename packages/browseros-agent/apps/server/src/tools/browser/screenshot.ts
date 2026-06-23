import type { Viewport } from '@browseros/cdp-protocol/domains/page'
import type { ProtocolApi } from '@browseros/cdp-protocol/protocol-api'
import { z } from 'zod'
import type { ScreenshotCaptureOptions } from '../../browser/core/screenshot'
import { defineTool } from './framework'

const DEFAULT_SCREENSHOT_FORMAT = 'jpeg'
const DEFAULT_SCREENSHOT_QUALITY = 80
const DEFAULT_SCREENSHOT_SIZE = { width: 1024, height: 768 } as const
const screenshotFormat = z.enum(['jpeg', 'png', 'webp'])
const screenshotSize = z.object({
  width: z.number().int().positive().max(4096).default(1024),
  height: z.number().int().positive().max(4096).default(768),
})

type ScreenshotFormat = z.infer<typeof screenshotFormat>
type ScreenshotSize = z.infer<typeof screenshotSize>

function screenshotQuality(format: ScreenshotFormat, quality?: number) {
  if (format !== 'jpeg') return undefined
  return quality ?? DEFAULT_SCREENSHOT_QUALITY
}

/** Builds a viewport clip that fits the capture within the requested target size. */
async function buildScreenshotClip(
  session: ProtocolApi,
  target: ScreenshotSize,
): Promise<Viewport> {
  const metrics = await session.Page.getLayoutMetrics()
  const viewport = metrics.cssLayoutViewport ?? metrics.layoutViewport
  const scale =
    viewport.clientWidth > 0 && viewport.clientHeight > 0
      ? Math.min(
          1,
          target.width / viewport.clientWidth,
          target.height / viewport.clientHeight,
        )
      : 1

  return {
    x: viewport.pageX,
    y: viewport.pageY,
    width: viewport.clientWidth,
    height: viewport.clientHeight,
    scale,
  }
}

export const screenshot = defineTool({
  name: 'screenshot',
  description:
    'Capture a screenshot of the page, returned inline. Defaults to JPEG quality 80 around 1024x768; prefer snapshot for structure/actions.',
  input: z.object({
    page: z.number().int(),
    format: screenshotFormat.default(DEFAULT_SCREENSHOT_FORMAT),
    quality: z.number().int().min(0).max(100).optional(),
    size: screenshotSize
      .optional()
      .describe('Max viewport capture size. Defaults to 1024x768.'),
    fullPage: z.boolean().optional().describe('Capture beyond the viewport.'),
    annotate: z
      .boolean()
      .optional()
      .describe('Overlay numbered refs from a fresh snapshot. Defaults true.'),
  }),
  annotations: { readOnlyHint: true },
  handler: async (args, ctx) => {
    const fullPage = args.fullPage ?? false
    const captureOptions: ScreenshotCaptureOptions = {
      format: args.format,
      fullPage,
      annotate: args.annotate ?? true,
    }
    const quality = screenshotQuality(args.format, args.quality)
    if (quality !== undefined) captureOptions.quality = quality
    if (!fullPage) {
      const { session } = await ctx.session.pages.getSession(args.page)
      captureOptions.clip = await buildScreenshotClip(
        session,
        args.size ?? DEFAULT_SCREENSHOT_SIZE,
      )
    }

    const result = await ctx.session.screenshot(args.page, captureOptions)
    return {
      content: [
        { type: 'image', data: result.data, mimeType: result.mimeType },
      ],
      ...(result.annotations.length > 0 && {
        structuredContent: {
          page: args.page,
          annotations: result.annotations,
        },
      }),
    }
  },
})
