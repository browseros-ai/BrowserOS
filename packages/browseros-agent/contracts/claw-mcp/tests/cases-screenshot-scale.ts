/** Density checks use a separate 2x browser launch. The image header is the
 * source of truth for pixels; annotation geometry is compared to CSS DOM boxes. */
import { expect } from 'bun:test'
import type { CaseContext, ContractCase } from './cases'
import { expectOk } from './helpers'
import { imageOf } from './mcp-client'
import { evalValue, runValue } from './sdk-helpers'

interface Box {
  x: number
  y: number
  width: number
  height: number
}
interface Annotation {
  name: string
  box: Box
  ref: string
}

/** Decode only the PNG IHDR / JPEG frame header; no image library can rescale it. */
function dimensions(data: string): { width: number; height: number } {
  const bytes = Buffer.from(data, 'base64')
  if (bytes.subarray(0, 8).toString('hex') === '89504e470d0a1a0a') {
    return { width: bytes.readUInt32BE(16), height: bytes.readUInt32BE(20) }
  }
  if (bytes.readUInt16BE(0) !== 0xffd8) throw new Error('expected PNG or JPEG')
  let offset = 2
  while (offset < bytes.length) {
    if (bytes[offset++] !== 0xff) throw new Error('malformed JPEG marker')
    while (bytes[offset] === 0xff) offset++
    const marker = bytes[offset++]
    const length = bytes.readUInt16BE(offset)
    if (marker !== undefined && [0xc0, 0xc1, 0xc2].includes(marker)) {
      return {
        height: bytes.readUInt16BE(offset + 3),
        width: bytes.readUInt16BE(offset + 5),
      }
    }
    offset += length
  }
  throw new Error('JPEG has no frame dimensions')
}

async function viewport(ctx: CaseContext, page: number) {
  const result = await evalValue<{
    width: number
    height: number
    dpr: number
  }>(
    ctx,
    page,
    'return { width: innerWidth, height: innerHeight, dpr: devicePixelRatio }',
  )
  // A 1x pass cannot distinguish the old default from the fix.
  expect(result.dpr).toBe(2)
  return result
}

/** Boxes are in image pixels: CSS boxes times `imageScale`, the image pixels per CSS pixel. */
async function assertAnnotations(
  ctx: CaseContext,
  page: number,
  annotations: Annotation[],
  imageScale = 1,
) {
  const boxes = await evalValue<Record<string, Box>>(
    ctx,
    page,
    `
    return Object.fromEntries(Array.from(document.querySelectorAll('button'), element => {
      const r = element.getBoundingClientRect()
      return [element.textContent, { x: r.x, y: r.y, width: r.width, height: r.height }]
    }))
  `,
  )
  expect(annotations).toHaveLength(Object.keys(boxes).length)
  for (const annotation of annotations) {
    expect(annotation.ref).toMatch(/^e\d+$/)
    const box = boxes[annotation.name]
    expect(box).toBeDefined()
    for (const axis of ['x', 'y', 'width', 'height'] as const) {
      expect(annotation.box[axis]).toBeCloseTo(box[axis] * imageScale, 0)
    }
  }
}

export const screenshotScaleCases: ContractCase[] = [
  {
    name: 'screenshot: default JPEG uses CSS pixels and device scale uses 2x pixels',
    browserArgs: ['--force-device-scale-factor=2'],
    async run(ctx) {
      const page = await ctx.openPage(ctx.fixture('/sdk-screenshot.html'))
      const { width, height, dpr } = await viewport(ctx, page)
      for (const scale of [undefined, 'device']) {
        const result = await ctx.mcp.callTool('screenshot', {
          page,
          ...(scale ? { scale } : {}),
        })
        expectOk(result)
        const image = imageOf(result)
        if (!image) throw new Error('screenshot omitted its image')
        expect(image.mimeType).toBe('image/jpeg')
        const ratio = scale === 'device' ? dpr : 1
        expect(dimensions(image.data)).toEqual({
          width: width * ratio,
          height: height * ratio,
        })
      }
    },
  },
  {
    name: 'screenshot: fullPage keeps device density and reaches past the viewport',
    browserArgs: ['--force-device-scale-factor=2'],
    async run(ctx) {
      const page = await ctx.openPage(ctx.fixture('/sdk-screenshot-tall.html'))
      const { dpr } = await viewport(ctx, page)
      const scrolled = await evalValue<{ width: number; height: number }>(
        ctx,
        page,
        'return { width: document.documentElement.scrollWidth, height: document.documentElement.scrollHeight }',
      )
      // scale is documented as ignored for fullPage, which keeps the display's density.
      const result = await ctx.mcp.callTool('screenshot', {
        page,
        fullPage: true,
      })
      expectOk(result)
      const image = imageOf(result)
      if (!image) throw new Error('screenshot omitted its image')
      expect(dimensions(image.data)).toEqual({
        width: scrolled.width * dpr,
        height: scrolled.height * dpr,
      })
    },
  },
  ...(['screenshot', 'run'] as const).map(
    (surface): ContractCase => ({
      name: `${surface}: default-scale screenshot annotations match CSS element boxes at 2x density`,
      browserArgs: ['--force-device-scale-factor=2'],
      async run(ctx) {
        const page = await ctx.openPage(ctx.fixture('/sdk-screenshot.html'))
        const { width, height } = await viewport(ctx, page)
        let annotations: Annotation[]
        if (surface === 'screenshot') {
          const result = await ctx.mcp.callTool('screenshot', {
            page,
            format: 'png',
            annotate: true,
          })
          expectOk(result)
          const image = imageOf(result)
          if (!image) throw new Error('annotated screenshot omitted its image')
          expect(dimensions(image.data)).toEqual({ width, height })
          // The hosted wire drops structured content for tools without an output schema, so
          // the boxes reach only the SDK; the granular surface proves the CSS-pixel image.
          return
        } else {
          // SDK screenshot exposes metadata and boxes, not image bytes. Pixel dimensions
          // are therefore verified at the granular surface that actually returns them.
          const result = await runValue<{
            annotations: Annotation[]
            bytes: number
          }>(
            ctx,
            page,
            'return await p.screenshot({ format: "png", annotate: true })',
          )
          expect(result.bytes).toBeGreaterThan(0)
          annotations = result.annotations
          const device = await runValue<{ bytes: number }>(
            ctx,
            page,
            'return await p.screenshot({ scale: "device" })',
          )
          expect(device.bytes).toBeGreaterThan(0)
        }
        await assertAnnotations(ctx, page, annotations)
      },
    }),
  ),
  ...(['{ scale: "device" }', '{ fullPage: true }'] as const).map(
    (options): ContractCase => ({
      name: `run: screenshot ${options} annotations are in device pixels at 2x density`,
      browserArgs: ['--force-device-scale-factor=2'],
      async run(ctx) {
        const page = await ctx.openPage(ctx.fixture('/sdk-screenshot.html'))
        const { dpr } = await viewport(ctx, page)
        const result = await runValue<{ annotations: Annotation[] }>(
          ctx,
          page,
          `return await p.screenshot({ ...${options}, format: "png", annotate: true })`,
        )
        await assertAnnotations(ctx, page, result.annotations, dpr)
      },
    }),
  ),
]
