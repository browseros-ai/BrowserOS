/**
 * screenshot (6), pdf (2), wait (4) and upload/download (4) cases.
 * Image formats are verified by their magic bytes and dimensions
 * parsed straight from the returned base64 — no image dependencies.
 */

import { mkdtemp, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import type { CaseContext, ContractCase } from './cases'
import { apiGet, expectError, expectOk, waitUntil } from './helpers'
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

  {
    name: 'upload: visible opener selects a hidden file input without extensions',
    async run(ctx) {
      const page = await ctx.openPage(ctx.fixture('/upload.html'))
      const snap = expectOk(await ctx.mcp.callTool('snapshot', { page }))
      const ref = snap
        .split('\n')
        .find((line) => line.includes('Attach document'))
        ?.match(/\[ref=(e\d+)\]/)?.[1]
      if (!ref) throw new Error(`no upload button in ${snap}`)
      const file = join(ctx.scratchDir, 'chooser-fixture.txt')
      await writeFile(file, 'selected through the intercepted chooser')
      expectOk(
        await ctx.mcp.callTool('upload', { page, ref, file }),
        'upload through opener',
      )
      const selected = expectOk(
        await ctx.mcp.callTool('evaluate_in_page', {
          page,
          code: 'return document.getElementById("hidden-names").textContent',
        }),
      )
      if (!selected.includes('chooser-fixture.txt'))
        throw new Error(`file was not selected: ${selected}`)
    },
  },

  {
    name: 'upload: an unexpected chooser is recoverable without a ref',
    async run(ctx) {
      const page = await ctx.openPage(ctx.fixture('/upload.html'))
      const snap = expectOk(await ctx.mcp.callTool('snapshot', { page }))
      const ref = snap
        .split('\n')
        .find((line) => line.includes('Attach document'))
        ?.match(/\[ref=(e\d+)\]/)?.[1]
      if (!ref) throw new Error('no upload button')
      const opened = await ctx.mcp.callTool('act', { page, kind: 'click', ref })
      expectOk(opened, 'click upload button')
      if (!textOf(opened).includes('"fileChooserOpen":true'))
        throw new Error(`no chooser marker: ${textOf(opened)}`)
      const file = join(ctx.scratchDir, 'pending-chooser.txt')
      await writeFile(file, 'pending chooser selection')
      expectOk(
        await ctx.mcp.callTool('upload', { page, file }),
        'complete pending chooser',
      )
      const selected = expectOk(
        await ctx.mcp.callTool('evaluate_in_page', {
          page,
          code: 'return document.getElementById("hidden-names").textContent',
        }),
      )
      if (!selected.includes('pending-chooser.txt'))
        throw new Error(`pending selection failed: ${selected}`)
      expectError(
        await ctx.mcp.callTool('upload', { page, file }),
        'already consumed chooser',
      )
      const nextSnapshot = expectOk(
        await ctx.mcp.callTool('snapshot', { page }),
      )
      const nextOpener = nextSnapshot
        .split('\n')
        .find((line) => line.includes('Attached pending-chooser.txt'))
        ?.match(/\[ref=(e\d+)\]/)?.[1]
      if (!nextOpener) throw new Error('no opener after selection')
      expectOk(
        await ctx.mcp.callTool('act', { page, kind: 'click', ref: nextOpener }),
      )
      expectOk(await ctx.mcp.callTool('upload', { page, files: [] }))
      const cleared = expectOk(
        await ctx.mcp.callTool('evaluate_in_page', {
          page,
          code: 'if (document.getElementById("hidden-file").files.length !== 0) throw new Error("files remain selected"); return "selection-cleared"',
        }),
      )
      if (!cleared.includes('selection-cleared'))
        throw new Error(`selection not cleared: ${cleared}`)
    },
  },
  {
    name: 'upload: run reports a chooser and finishes it through the same interface',
    async run(ctx) {
      const page = await ctx.openPage(ctx.fixture('/upload.html'))
      const snap = expectOk(await ctx.mcp.callTool('snapshot', { page }))
      const ref = snap
        .split('\n')
        .find((line) => line.includes('Attach document'))
        ?.match(/\[ref=(e\d+)\]/)?.[1]
      if (!ref) throw new Error('no upload button')
      const file = join(ctx.scratchDir, 'script-chooser.txt')
      await writeFile(file, 'script selection')
      const result = expectOk(
        await ctx.mcp.callTool('run', {
          code: `const p = browser.page(${page}); const opened = await p.click(${JSON.stringify(ref)}); if (!opened?.fileChooserOpen) throw new Error('missing chooser marker'); await p.upload(null, [${JSON.stringify(file)}]); return 'selection complete';`,
        }),
        'script upload',
      )
      if (!result.includes('selection complete')) throw new Error(result)
      await waitUntil(async () => {
        const detail = await apiGet(
          ctx.server,
          `/api/v1/sessions/${ctx.mcp.sessionId}`,
        )
        if (!detail.ok) return false
        const body: {
          dispatches?: Array<{
            toolName: string
            pageId?: number
            resultMeta?: string
          }>
        } = await detail.json()
        return (
          Array.isArray(body.dispatches) &&
          body.dispatches.some(
            (row) =>
              row.toolName === 'input.click' &&
              row.pageId === page &&
              JSON.parse(row.resultMeta ?? '{}').fileChooserOpen === true,
          )
        )
      }, 'the cockpit audit to preserve the script chooser marker')
    },
  },
  {
    name: 'upload: navigation invalidates a pending chooser',
    async run(ctx) {
      const page = await ctx.openPage(ctx.fixture('/upload.html'))
      const snap = expectOk(await ctx.mcp.callTool('snapshot', { page }))
      const ref = snap
        .split('\n')
        .find((line) => line.includes('Attach document'))
        ?.match(/\[ref=(e\d+)\]/)?.[1]
      if (!ref) throw new Error('no upload button')
      const opened = await ctx.mcp.callTool('act', { page, kind: 'click', ref })
      expectOk(opened)
      if (!textOf(opened).includes('"fileChooserOpen":true'))
        throw new Error('chooser was not opened before navigation')
      expectOk(
        await ctx.mcp.callTool('navigate', {
          page,
          url: ctx.fixture('/upload.html'),
        }),
      )
      const file = join(ctx.scratchDir, 'stale-chooser.txt')
      await writeFile(file, 'must not reach the replacement document')
      expectError(
        await ctx.mcp.callTool('upload', { page, file }),
        'stale chooser',
      )
    },
  },

  {
    name: 'upload: a cross-origin frame chooser uses its own document session',
    async run(ctx) {
      const child = ctx
        .fixture2('/upload.html')
        .replace('127.0.0.1', 'localhost')
      const page = await ctx.openPage(
        `${ctx.fixture('/upload-frame.html')}?src=${encodeURIComponent(child)}`,
      )
      let ref: string | undefined
      await waitUntil(async () => {
        const snap = expectOk(await ctx.mcp.callTool('snapshot', { page }))
        ref = snap
          .split('\n')
          .find((line) => line.includes('Attach document'))
          ?.match(/\[ref=(e\d+)\]/)?.[1]
        return Boolean(ref)
      }, 'cross-origin upload button')
      const opened = await ctx.mcp.callTool('act', { page, kind: 'click', ref })
      expectOk(opened, 'open frame chooser')
      if (!textOf(opened).includes('"fileChooserOpen":true'))
        throw new Error(`frame chooser missing: ${textOf(opened)}`)
      const file = join(ctx.scratchDir, 'frame-chooser.txt')
      await writeFile(file, 'selected in the remote child document')
      expectOk(
        await ctx.mcp.callTool('upload', { page, file }),
        'select file in frame',
      )
      await waitUntil(async () => {
        const selected = expectOk(await ctx.mcp.callTool('snapshot', { page }))
        return selected.includes('Attached frame-chooser.txt')
      }, 'the remote document to confirm its selected file')
    },
  },
  {
    name: 'upload: a borrowed iframe can open a parent-page picker',
    async run(ctx) {
      const child = ctx
        .fixture2('/upload.html?parentPicker=1')
        .replace('127.0.0.1', 'localhost')
      const url = `${ctx.fixture('/upload-frame.html')}?src=${encodeURIComponent(child)}&borrowed=1`
      // Create through the browser's debugging endpoint, outside MCP ownership. This is a
      // borrowed user tab: explicit upload must lease the whole page's picker interception.
      const created = await fetch(
        `http://127.0.0.1:${ctx.browser.cdpPort}/json/new?${encodeURIComponent(url)}`,
        { method: 'PUT' },
      )
      if (!created.ok)
        throw new Error(`external tab creation failed: ${created.status}`)
      const target: { id: string } = await created.json()
      let page: number | undefined
      try {
        await waitUntil(async () => {
          const tabs = expectOk(
            await ctx.mcp.callTool('tabs', { action: 'list' }),
          )
          const id = tabs
            .split('\n')
            .find((line) => line.includes(url))
            ?.match(/^\[(\d+)\]/)?.[1]
          page = id ? Number(id) : undefined
          return page !== undefined
        }, 'the borrowed tab to appear')
        let ref: string | undefined
        await waitUntil(async () => {
          const snap = expectOk(await ctx.mcp.callTool('snapshot', { page }))
          ref = snap
            .split('\n')
            .find((line) => line.includes('Attach to parent'))
            ?.match(/\[ref=(e\d+)\]/)?.[1]
          return Boolean(ref)
        }, 'the borrowed frame upload button')
        const file = join(ctx.scratchDir, 'parent-chooser.txt')
        await writeFile(file, 'selected by the borrowed page parent')
        expectOk(
          await ctx.mcp.callTool('upload', { page, ref, file }),
          'select through the parent picker',
        )
        const selected = expectOk(await ctx.mcp.callTool('snapshot', { page }))
        if (!selected.includes('Parent selection: parent-chooser.txt'))
          throw new Error(`parent input was not selected: ${selected}`)
      } finally {
        if (page !== undefined)
          await ctx.mcp.callTool('tabs', { action: 'close', page })
        else
          await fetch(
            `http://127.0.0.1:${ctx.browser.cdpPort}/json/close/${target.id}`,
          )
      }
    },
  },
  {
    name: 'run: ordinary hovers do not pay a chooser delay',
    async run(ctx) {
      const page = await ctx.openPage(ctx.fixture('/upload.html'))
      const snap = expectOk(await ctx.mcp.callTool('snapshot', { page }))
      const ref = snap
        .split('\n')
        .find((line) => line.includes('Attach document'))
        ?.match(/\[ref=(e\d+)\]/)?.[1]
      if (!ref) throw new Error('no hover target')
      const result = expectOk(
        await ctx.mcp.callTool('run', {
          // The old 100ms wait alone costs ten seconds for this batch. Leave several
          // seconds for real CDP work while staying below that unavoidable old minimum.
          timeout: 9500,
          code: `const p = browser.page(${page}); for (let i = 0; i < 100; i++) await p.hover(${JSON.stringify(ref)}); return 'hover-batch-complete';`,
        }),
        'ordinary input batch',
      )
      if (!result.includes('hover-batch-complete'))
        throw new Error(`ordinary input batch exceeded its budget: ${result}`)
    },
  },
  {
    name: 'upload: single-file chooser rejects multiple paths and remains retryable',
    async run(ctx) {
      const page = await ctx.openPage(ctx.fixture('/upload.html'))
      const snap = expectOk(await ctx.mcp.callTool('snapshot', { page }))
      const ref = snap
        .split('\n')
        .find((line) => line.includes('Attach document'))
        ?.match(/\[ref=(e\d+)\]/)?.[1]
      if (!ref) throw new Error('no upload button')
      expectOk(await ctx.mcp.callTool('act', { page, kind: 'click', ref }))
      const first = join(ctx.scratchDir, 'single-first.txt')
      const second = join(ctx.scratchDir, 'single-second.txt')
      await writeFile(first, 'first')
      await writeFile(second, 'second')
      expectError(
        await ctx.mcp.callTool('upload', { page, files: [first, second] }),
        'multiple paths for single-file chooser',
      )
      expectOk(
        await ctx.mcp.callTool('upload', { page, files: [first] }),
        'retry single file',
      )
    },
  },

  {
    name: 'upload: a removed chooser input fails and releases the pending request',
    async run(ctx) {
      const page = await ctx.openPage(ctx.fixture('/upload.html'))
      const snap = expectOk(await ctx.mcp.callTool('snapshot', { page }))
      const ref = snap
        .split('\n')
        .find((line) => line.includes('Attach document'))
        ?.match(/\[ref=(e\d+)\]/)?.[1]
      if (!ref) throw new Error('no upload button')
      const opened = await ctx.mcp.callTool('act', { page, kind: 'click', ref })
      if (!expectOk(opened).includes('"fileChooserOpen":true'))
        throw new Error('no pending chooser')
      expectOk(
        await ctx.mcp.callTool('evaluate_in_page', {
          page,
          code: 'document.getElementById("hidden-file").remove(); return true',
        }),
      )
      const file = join(ctx.scratchDir, 'replacement-input.txt')
      await writeFile(file, 'must go only to the replacement input')
      expectError(
        await ctx.mcp.callTool('upload', { page, file }),
        'removed chooser input',
      )
      expectOk(
        await ctx.mcp.callTool('evaluate_in_page', {
          page,
          code: 'const input = document.createElement("input"); input.type="file"; input.id="hidden-file"; input.hidden=true; document.body.appendChild(input); return true',
        }),
      )
      const fresh = expectOk(await ctx.mcp.callTool('snapshot', { page }))
      const opener = fresh
        .split('\n')
        .find((line) => line.includes('Attach document'))
        ?.match(/\[ref=(e\d+)\]/)?.[1]
      if (!opener) throw new Error('no replacement opener')
      expectOk(
        await ctx.mcp.callTool('upload', { page, ref: opener, file }),
        'retry replacement input',
      )
    },
  },

  {
    name: 'download: the click can create its initiating frame',
    async run(ctx) {
      const page = await ctx.openPage(ctx.fixture('/links.html'))
      const snap = expectOk(await ctx.mcp.callTool('snapshot', { page }))
      const ref = snap
        .split('\n')
        .find((line) => line.includes('Download through new frame'))
        ?.match(/\[ref=(e\d+)\]/)?.[1]
      if (!ref) throw new Error('no frame download button')
      const text = expectOk(
        await ctx.mcp.callTool('download', { page, ref }),
        'download through new frame',
      )
      const path = text.match(/to: (.+)$/m)?.[1]
      if (!path || !(await Bun.file(path).text()).includes('fixture report'))
        throw new Error(`frame download missing: ${text}`)
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
            await ctx.mcp.callTool('evaluate_in_page', {
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
          await ctx.mcp.callTool('evaluate_in_page', {
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
      // Anchored on the trailing path rather than a whitespace run: a download
      // keeps the name the site chose, which can contain spaces.
      const path = text.match(/to: (.+)$/m)?.[1]
      if (!path || !(await Bun.file(path).exists())) {
        throw new Error(`download did not land a readable file: ${path}`)
      }
      if (!path.includes('/Downloads/')) {
        throw new Error(`download did not land in the download folder: ${path}`)
      }
      const contents = await Bun.file(path).text()
      if (!contents.includes('fixture report')) {
        throw new Error(`downloaded file had unexpected contents: ${contents}`)
      }
      const reported = Number(text.match(/\((\d+) bytes\)/)?.[1])
      const size = Bun.file(path).size
      if (reported !== size) {
        throw new Error(`reported ${reported} bytes for a ${size} byte file`)
      }
    },
  },
  {
    name: 'download: a second download renames around the first',
    async run(ctx) {
      // A managed download writes straight to the path it is given and replaces
      // what is there, so the file goes through a staging directory and is moved
      // under a free name. Without that, this second download would destroy the
      // first file and report a path whose contents are not what it fetched.
      const page = await ctx.openPage(ctx.fixture('/links.html'))
      const paths: string[] = []
      for (const attempt of ['first', 'second']) {
        const snap = expectOk(await ctx.mcp.callTool('snapshot', { page }))
        const ref = snap
          .split('\n')
          .find((line) => line.includes('Download report'))
          ?.match(/\[ref=(e\d+)\]/)?.[1]
        if (!ref) throw new Error(`no download ref on the ${attempt} attempt`)
        const text = expectOk(
          await ctx.mcp.callTool('download', { page, ref }),
          `download ${attempt}`,
        )
        const path = text.match(/to: (.+)$/m)?.[1]
        if (!path) throw new Error(`no path on the ${attempt} attempt: ${text}`)
        paths.push(path)
      }
      const [first, second] = paths
      if (first === second) {
        throw new Error(`both downloads reported the same path: ${first}`)
      }
      // A counted name, not a specific counter: an earlier case in this suite
      // has already taken "report.txt", so the pair here starts further along.
      if (!/report \(\d+\)\.txt$/.test(second)) {
        throw new Error(
          `second download was not renamed around the first: ${second}`,
        )
      }
      for (const path of paths) {
        if (!(await Bun.file(path).exists())) {
          throw new Error(
            `${path} is missing, so one download replaced the other`,
          )
        }
        const contents = await Bun.file(path).text()
        if (!contents.includes('fixture report')) {
          throw new Error(`${path} had unexpected contents: ${contents}`)
        }
      }
    },
  },
  {
    name: "download: the browser's own list reports it accurately",
    async run(ctx) {
      // The point of letting the browser choose the destination. When the file
      // was staged elsewhere and moved, the browser kept recording the staging
      // path, so every row read "Deleted" with no reveal action while the file
      // sat in the download folder.
      const page = await ctx.openPage(ctx.fixture('/links.html'))
      const snap = expectOk(await ctx.mcp.callTool('snapshot', { page }))
      const ref = snap
        .split('\n')
        .find((line) => line.includes('Download report'))
        ?.match(/\[ref=(e\d+)\]/)?.[1]
      if (!ref) throw new Error('no download ref')
      const text = expectOk(
        await ctx.mcp.callTool('download', { page, ref }),
        'download',
      )
      const path = text.match(/to: (.+)$/m)?.[1]
      if (!path) throw new Error(`no path reported: ${text}`)
      const name = path.split('/').pop() ?? ''

      expectOk(
        await ctx.mcp.callTool('navigate', {
          page,
          action: 'url',
          url: 'chrome://downloads',
        }),
        'open the download list',
      )
      const list = expectOk(
        await ctx.mcp.callTool('snapshot', { page, mode: 'interactive' }),
      )
      const row = list.split('\n').find((line) => line.includes(name))
      if (!row) {
        throw new Error(`no list row for ${name} in:\n${list.slice(0, 600)}`)
      }
      if (row.includes('Deleted')) {
        throw new Error(`the list says the file is gone: ${row.trim()}`)
      }
      // The reveal action is labelled per platform: "Show in Finder" on macOS,
      // "Show in folder" elsewhere. Its presence is the point, not its wording.
      if (!/Show in (Finder|folder)/.test(row)) {
        throw new Error(`the list cannot reveal the file: ${row.trim()}`)
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
