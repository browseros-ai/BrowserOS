/**
 * screenshot (6), pdf (2), wait (4) and upload/download (4) cases.
 * Image formats are verified by their magic bytes and dimensions
 * parsed straight from the returned base64 — no image dependencies.
 */

import { mkdtemp, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import type { CaseContext, ContractCase } from './cases'
import { expectError, expectOk, waitUntil } from './helpers'
import { imageOf, textOf } from './mcp-client'

function magicHex(base64: string, bytes: number): string {
  return Buffer.from(base64, 'base64').subarray(0, bytes).toString('hex')
}

/** PNG width/height live at byte offsets 16/20 (big-endian) in the IHDR chunk. */
function pngDimensions(base64: string): { width: number; height: number } {
  const buffer = Buffer.from(base64, 'base64')
  return { width: buffer.readUInt32BE(16), height: buffer.readUInt32BE(20) }
}

function requireImage(
  result: Awaited<ReturnType<CaseContext['mcp']['callTool']>>,
  context: string,
): { data: string; mimeType: string } {
  const image = imageOf(result)
  if (!image) throw new Error(`${context} returned no image block`)
  return image
}

function spillPath(text: string): string {
  const path = text.match(/to: (\S+)/)?.[1]
  if (!path) throw new Error(`no output path in: ${text.slice(0, 200)}`)
  return path
}

export const captureIoCases: ContractCase[] = [
  // screenshot -------------------------------------------------------------
  {
    name: 'screenshot: jpeg carries the JPEG magic bytes',
    smoke: true,
    async run(ctx) {
      const page = await ctx.openPage(ctx.fixture('/media.html'))
      const image = requireImage(
        await ctx.mcp.callTool('screenshot', { page, format: 'jpeg' }),
        'jpeg screenshot',
      )
      if (image.mimeType !== 'image/jpeg') {
        throw new Error(`jpeg screenshot had ${image.mimeType}`)
      }
      if (!magicHex(image.data, 2).startsWith('ffd8')) {
        throw new Error(
          `jpeg screenshot lacked FFD8: ${magicHex(image.data, 4)}`,
        )
      }
    },
  },
  {
    name: 'screenshot: png carries the PNG magic bytes',
    async run(ctx) {
      const page = await ctx.openPage(ctx.fixture('/media.html'))
      const image = requireImage(
        await ctx.mcp.callTool('screenshot', { page, format: 'png' }),
        'png screenshot',
      )
      if (image.mimeType !== 'image/png') {
        throw new Error(`png screenshot had ${image.mimeType}`)
      }
      if (!magicHex(image.data, 4).startsWith('89504e47')) {
        throw new Error(
          `png screenshot lacked 8950: ${magicHex(image.data, 4)}`,
        )
      }
    },
  },
  {
    name: 'screenshot: webp carries the RIFF magic bytes',
    async run(ctx) {
      const page = await ctx.openPage(ctx.fixture('/media.html'))
      const image = requireImage(
        await ctx.mcp.callTool('screenshot', { page, format: 'webp' }),
        'webp screenshot',
      )
      // RIFF container: bytes 0-3 are "RIFF" (52494646).
      if (!magicHex(image.data, 4).startsWith('52494646')) {
        throw new Error(
          `webp screenshot lacked RIFF: ${magicHex(image.data, 4)}`,
        )
      }
    },
  },
  {
    name: 'screenshot: size parameter changes the captured dimensions',
    async run(ctx) {
      const page = await ctx.openPage(ctx.fixture('/media.html'))
      const small = pngDimensions(
        requireImage(
          await ctx.mcp.callTool('screenshot', {
            page,
            format: 'png',
            size: { width: 400, height: 300 },
          }),
          'small screenshot',
        ).data,
      )
      const large = pngDimensions(
        requireImage(
          await ctx.mcp.callTool('screenshot', {
            page,
            format: 'png',
            size: { width: 900, height: 600 },
          }),
          'large screenshot',
        ).data,
      )
      if (large.width <= small.width || large.height <= small.height) {
        throw new Error(
          `size did not scale the capture: small ${JSON.stringify(small)} large ${JSON.stringify(large)}`,
        )
      }
    },
  },
  {
    name: 'screenshot: fullPage is taller than the viewport',
    async run(ctx) {
      const page = await ctx.openPage(ctx.fixture('/media.html'))
      const viewport = pngDimensions(
        requireImage(
          await ctx.mcp.callTool('screenshot', {
            page,
            format: 'png',
            size: { width: 800, height: 600 },
          }),
          'viewport screenshot',
        ).data,
      )
      const full = pngDimensions(
        requireImage(
          await ctx.mcp.callTool('screenshot', {
            page,
            format: 'png',
            fullPage: true,
          }),
          'fullPage screenshot',
        ).data,
      )
      // media.html is three 800px blocks: fullPage must dwarf a viewport.
      if (full.height <= viewport.height) {
        throw new Error(
          `fullPage (${full.height}) not taller than viewport (${viewport.height})`,
        )
      }
    },
  },
  {
    name: 'screenshot: annotate returns an image',
    async run(ctx) {
      const page = await ctx.openPage(ctx.fixture('/form.html'))
      await ctx.mcp.callTool('snapshot', { page })
      const image = requireImage(
        await ctx.mcp.callTool('screenshot', {
          page,
          format: 'png',
          annotate: true,
        }),
        'annotated screenshot',
      )
      if (image.mimeType !== 'image/png') {
        throw new Error(`annotated screenshot had ${image.mimeType}`)
      }
    },
  },

  // pdf --------------------------------------------------------------------
  {
    name: 'pdf: renders a file with the %PDF magic',
    async run(ctx) {
      const page = await ctx.openPage(ctx.fixture('/links.html'))
      const text = expectOk(await ctx.mcp.callTool('pdf', { page }), 'pdf')
      const path = spillPath(text)
      const bytes = await Bun.file(path).bytes()
      const magic = Buffer.from(bytes.subarray(0, 5)).toString()
      if (magic !== '%PDF-') {
        throw new Error(`pdf file lacked the %PDF magic: ${magic}`)
      }
    },
  },
  {
    name: 'pdf: landscape orientation is accepted',
    async run(ctx) {
      const page = await ctx.openPage(ctx.fixture('/links.html'))
      const text = expectOk(
        await ctx.mcp.callTool('pdf', { page, landscape: true }),
        'pdf landscape',
      )
      const bytes = await Bun.file(spillPath(text)).bytes()
      if (Buffer.from(bytes.subarray(0, 5)).toString() !== '%PDF-') {
        throw new Error('landscape PDF lacked the %PDF magic')
      }
    },
  },

  // wait -------------------------------------------------------------------
  {
    name: 'wait: for=time elapses at least the requested duration',
    async run(ctx) {
      const page = await ctx.openPage(ctx.fixture('/form.html'))
      const started = Date.now()
      expectOk(
        await ctx.mcp.callTool('wait', { page, for: 'time', value: 800 }),
        'wait time',
      )
      const elapsed = Date.now() - started
      if (elapsed < 700) {
        throw new Error(`wait time returned too early: ${elapsed}ms`)
      }
    },
  },
  {
    name: 'wait: for=text resolves when delayed text appears',
    async run(ctx) {
      const page = await ctx.openPage(ctx.fixture('/dynamic.html'))
      const text = expectOk(
        await ctx.mcp.callTool('wait', {
          page,
          for: 'text',
          value: 'delayed content ready',
          timeout: 5_000,
        }),
        'wait for text',
      )
      if (!/matched/i.test(text)) {
        throw new Error(`wait for text did not report a match: ${text}`)
      }
    },
  },
  {
    name: 'wait: for=selector resolves on a present selector',
    async run(ctx) {
      const page = await ctx.openPage(ctx.fixture('/dynamic.html'))
      const text = expectOk(
        await ctx.mcp.callTool('wait', {
          page,
          for: 'selector',
          value: '#marker',
          timeout: 3_000,
        }),
        'wait for selector',
      )
      if (!/matched/i.test(text)) {
        throw new Error(`wait for selector did not report a match: ${text}`)
      }
    },
  },
  {
    name: 'wait: timeout expiry has a recognizable shape',
    async run(ctx) {
      const page = await ctx.openPage(ctx.fixture('/form.html'))
      const result = await ctx.mcp.callTool('wait', {
        page,
        for: 'text',
        value: 'this text never appears xyzzy',
        timeout: 1_000,
      })
      const text = textOf(result)
      if (!/timed out/i.test(text)) {
        throw new Error(
          `wait timeout did not report a timeout: ${text.slice(0, 120)}`,
        )
      }
    },
  },

  // upload / download ------------------------------------------------------
  {
    name: 'upload: single file name is reported by the page',
    async run(ctx) {
      const page = await ctx.openPage(ctx.fixture('/upload.html'))
      const snap = expectOk(await ctx.mcp.callTool('snapshot', { page }))
      const ref = snap
        .split('\n')
        .find((line) => line.includes('Single upload'))
        ?.match(/\[ref=(e\d+)\]/)?.[1]
      if (!ref)
        throw new Error(`no single-upload ref in:\n${snap.slice(0, 300)}`)
      const dir = await mkdtemp(join(tmpdir(), 'claw-upload-'))
      const file = join(dir, 'single-fixture.txt')
      await writeFile(file, 'single upload contents')
      expectOk(
        await ctx.mcp.callTool('upload', { page, ref, file }),
        'upload single',
      )
      await waitUntil(
        async () =>
          textOf(
            await ctx.mcp.callTool('evaluate', {
              page,
              code: 'return document.getElementById("single-names").textContent',
            }),
          ).includes('single-fixture.txt'),
        'the page to report the uploaded file name',
      )
    },
  },
  {
    name: 'upload: multiple file names are reported by the page',
    async run(ctx) {
      const page = await ctx.openPage(ctx.fixture('/upload.html'))
      const snap = expectOk(await ctx.mcp.callTool('snapshot', { page }))
      const ref = snap
        .split('\n')
        .find((line) => line.includes('Multi upload'))
        ?.match(/\[ref=(e\d+)\]/)?.[1]
      if (!ref)
        throw new Error(`no multi-upload ref in:\n${snap.slice(0, 300)}`)
      const dir = await mkdtemp(join(tmpdir(), 'claw-upload-'))
      const first = join(dir, 'multi-one.txt')
      const second = join(dir, 'multi-two.txt')
      await writeFile(first, 'one')
      await writeFile(second, 'two')
      expectOk(
        await ctx.mcp.callTool('upload', { page, ref, files: [first, second] }),
        'upload multi',
      )
      await waitUntil(async () => {
        const names = textOf(
          await ctx.mcp.callTool('evaluate', {
            page,
            code: 'return document.getElementById("multi-names").textContent',
          }),
        )
        return (
          names.includes('multi-one.txt') && names.includes('multi-two.txt')
        )
      }, 'the page to report both uploaded file names')
    },
  },
  {
    name: 'download: file lands on disk and its path is reported',
    async run(ctx) {
      const page = await ctx.openPage(ctx.fixture('/links.html'))
      const snap = expectOk(await ctx.mcp.callTool('snapshot', { page }))
      const ref = snap
        .split('\n')
        .find((line) => line.includes('Download report'))
        ?.match(/\[ref=(e\d+)\]/)?.[1]
      if (!ref) throw new Error(`no download ref in:\n${snap.slice(0, 300)}`)
      const text = expectOk(
        await ctx.mcp.callTool('download', { page, ref }),
        'download',
      )
      const path = text.match(/to: (\S+)/)?.[1]
      if (!path || !(await Bun.file(path).exists())) {
        throw new Error(`download did not land a readable file: ${path}`)
      }
      const contents = await Bun.file(path).text()
      if (!contents.includes('fixture report')) {
        throw new Error(`downloaded file had unexpected contents: ${contents}`)
      }
    },
  },
  {
    name: 'download: a bad ref errors',
    async run(ctx) {
      const page = await ctx.openPage(ctx.fixture('/links.html'))
      await ctx.mcp.callTool('snapshot', { page })
      expectError(
        await ctx.mcp.callTool('download', { page, ref: 'e999' }),
        'download bad ref',
      )
    },
  },
]
