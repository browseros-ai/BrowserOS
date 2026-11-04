/** Saved output contracts cross the real MCP wire; no host filesystem reads are used. */
import { expect } from 'bun:test'
import type { CaseContext, ContractCase } from './cases'
import { apiGet, expectError, expectOk, waitUntil } from './helpers'
import { imageOf, type McpToolResult, readRpcResponse } from './mcp-client'
import { startRustServer } from './rust-server'

interface SavedOutput {
  id: string
  parts: { name: string; mimeType: string; bytes: number }[]
}

// Each call is a fresh HTTP request without transport session state. Only Neo's
// logical handle links the producer and reader, as a hosted personal agent does.
async function statelessCall(
  ctx: CaseContext,
  name: string,
  args: Record<string, unknown>,
) {
  const response = await fetch(`${ctx.server.baseUrl}/mcp`, {
    method: 'POST',
    headers: {
      'content-type': 'application/json',
      accept: 'application/json, text/event-stream',
      'mcp-protocol-version': '2026-07-28',
      'mcp-method': 'tools/call',
      'mcp-name': name,
    },
    body: JSON.stringify({
      jsonrpc: '2.0',
      id: 1,
      method: 'tools/call',
      params: {
        _meta: {
          'io.modelcontextprotocol/protocolVersion': '2026-07-28',
          'io.modelcontextprotocol/clientInfo': {
            name: 'output-stateless',
            version: '1.0',
          },
          'io.modelcontextprotocol/clientCapabilities': {},
        },
        name,
        arguments: args,
      },
    }),
  })
  if (!response.ok)
    throw new Error(
      `stateless call failed: ${response.status} ${await response.text()}`,
    )
  const envelope = await readRpcResponse(response, 1)
  if (envelope.error) throw new Error(envelope.error.message)
  return envelope.result as McpToolResult & { _meta?: Record<string, unknown> }
}

function outputOf(result: McpToolResult): SavedOutput {
  expectOk(result)
  const output = result.structuredContent?.output as SavedOutput | undefined
  if (!output?.id || !Array.isArray(output.parts))
    throw new Error('tool omitted its saved output descriptor')
  return output
}

async function bytesOf(
  ctx: CaseContext,
  outputId: string,
  part: string,
): Promise<Buffer> {
  const chunks: Buffer[] = []
  let offset = 0
  for (;;) {
    const result = await ctx.mcp.callTool('read_output', {
      outputId,
      part,
      offset,
      limit: 4096,
      encoding: 'base64',
    })
    expectOk(result)
    const chunk = result.structuredContent
    if (typeof chunk?.data !== 'string') throw new Error('missing binary chunk')
    chunks.push(Buffer.from(chunk.data, 'base64'))
    if (chunk.eof === true) return Buffer.concat(chunks)
    if (typeof chunk.nextOffset !== 'number' || chunk.nextOffset <= offset)
      throw new Error('output reader did not advance')
    offset = chunk.nextOffset
  }
}

export const outputCases: ContractCase[] = [
  {
    name: 'outputs: total nested storage failure reports persistence loss without replaying the action',
    async run(ctx) {
      const server = await startRustServer(ctx.browser.cdpPort, {
        env: { BROWSEROS_OUTPUT_DIR: '/dev/null/neo-output' },
      })
      const isolated = { ...ctx, server }
      let page: number | undefined
      let session: string | undefined
      try {
        await waitUntil(
          async () =>
            !(await statelessCall(isolated, 'tabs', { action: 'list' }))
              .isError,
          'isolated failure server to attach',
        )
        const opened = await statelessCall(isolated, 'tabs', {
          action: 'new',
          url: ctx.fixture('/media.html'),
        })
        expectOk(opened)
        const handle = opened._meta?.['com.browseros.neo/session']
        if (
          typeof handle !== 'string' ||
          typeof opened.structuredContent?.page !== 'number'
        )
          throw new Error('missing isolated page/session')
        session = handle
        page = opened.structuredContent.page
        const result = await statelessCall(isolated, 'run', {
          session,
          code: `await browser.page(${page}).screenshot(); return {done:true}`,
        })
        expect(expectOk(result)).toContain('No output parts could be retained')
        expect(result.structuredContent?.value).toEqual({ done: true })
        expect(result.structuredContent?.outputWriteFailed).toBe(true)
        expect(result.structuredContent).not.toHaveProperty('output')
        expect(imageOf(result)).toBeDefined()
      } finally {
        if (page !== undefined && session)
          await statelessCall(isolated, 'tabs', {
            action: 'close',
            page,
            session,
          }).catch(() => {})
        await server.stop()
      }
    },
  },
  {
    name: 'outputs: nested screenshot previews are bounded and extra captures stay readable',
    // Linux new-headless stalls on repeated background captures. Hosted Neo
    // uses a headed X server; exercise that rendering path for this visual batch.
    headless: process.platform === 'linux' ? false : undefined,
    async run(ctx) {
      const page = await ctx.openPage(ctx.fixture('/media.html'))
      const result = await ctx.mcp.callTool('run', {
        // Keep native work well inside run's execution budget on software-rendered
        // Linux. The public Rust store tests exercise the separate 64-part boundary.
        code: `const p=browser.page(${page}); for(let i=0;i<6;i++) await p.screenshot({format:'png'}); return {captures:6}`,
      })
      const output = outputOf(result)
      expect(
        result.content.filter((block) => block.type === 'image').length,
      ).toBeLessThanOrEqual(4)
      expect(imageOf(result)).toBeDefined()
      expect(output.parts.length).toBe(13)
      expect(
        output.parts.filter((part) => part.mimeType === 'image/png').length,
      ).toBe(6)
      expect(result.structuredContent).not.toHaveProperty('outputWriteFailed')
      expect(
        (await bytesOf(ctx, output.id, 'screenshot-6.png')).subarray(0, 8),
      ).toEqual(Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]))
      expect(
        JSON.parse((await bytesOf(ctx, output.id, 'result.json')).toString())
          .structuredContent.value,
      ).toEqual({ captures: 6 })
    },
  },
  {
    name: 'outputs: expired live handles retain output through their canonical task',
    async run(ctx) {
      const server = await startRustServer(ctx.browser.cdpPort, {
        env: {
          CLAW_SESSION_IDLE_MS: '1000',
          CLAW_SESSION_SWEEP_INTERVAL_MS: '100',
        },
      })
      const isolated = { ...ctx, server }
      try {
        await waitUntil(
          async () =>
            !(await statelessCall(isolated, 'tabs', { action: 'list' }))
              .isError,
          'isolated server to attach',
        )
        const produced = await statelessCall(isolated, 'run', {
          code: "return 'retained value '.repeat(3000)",
        })
        const output = outputOf(produced)
        const session = produced._meta?.['com.browseros.neo/session']
        if (typeof session !== 'string')
          throw new Error('missing logical session')
        await waitUntil(async () => {
          const response = await apiGet(server, `/api/v1/sessions/${session}`)
          if (!response.ok) return false
          const detail = (await response.json()) as {
            session?: { status?: string }
            summary?: { status?: string }
          }
          const status = detail.session?.status ?? detail.summary?.status
          return status !== undefined && status !== 'live'
        }, 'original live session to expire')
        const recovered = await statelessCall(isolated, 'read_output', {
          session,
          outputId: output.id,
          part: 'result.json',
          limit: 512,
        })
        expectOk(recovered)
        expect(recovered._meta?.['com.browseros.neo/session']).not.toBe(session)
        expect(recovered.structuredContent?.text).toContain('retained value ')
      } finally {
        await server.stop()
      }
    },
  },
  {
    name: 'outputs: downloads are retrievable by a content-only client',
    async run(ctx) {
      const page = await ctx.openPage(ctx.fixture('/links.html'))
      const snapshot = expectOk(await ctx.mcp.callTool('snapshot', { page }))
      const ref = snapshot
        .split('\n')
        .find((line) => line.includes('Download report'))
        ?.match(/\[ref=(e\d+)\]/)?.[1]
      if (!ref) throw new Error('missing download link')
      const output = outputOf(await ctx.mcp.callTool('download', { page, ref }))
      const chunks: Buffer[] = []
      let offset = 0
      for (;;) {
        const result = await ctx.mcp.callTool('read_output', {
          outputId: output.id,
          part: 'download.txt',
          encoding: 'base64',
          offset,
          limit: 4,
        })
        expectOk(result)
        // The host may append session hints as separate content blocks. A client
        // without structuredContent still receives the reader's complete JSON block.
        const payload = result.content.find(
          (block) => block.type === 'text' && block.text?.startsWith('{'),
        )?.text
        if (!payload) throw new Error('missing content-only output chunk')
        const chunk = JSON.parse(payload)
        expect(typeof chunk.data).toBe('string')
        chunks.push(Buffer.from(chunk.data, 'base64'))
        if (chunk.eof) break
        expect(chunk.nextOffset).toBeGreaterThan(offset)
        offset = chunk.nextOffset
      }
      expect(Buffer.concat(chunks).toString()).toBe('fixture report contents\n')
    },
  },
  {
    name: 'outputs: automatic action diffs share the saved action result',
    async run(ctx) {
      const page = await ctx.openPage(ctx.fixture('/output-large.html'))
      const matches = expectOk(
        await ctx.mcp.callTool('grep', { page, pattern: 'Change all labels' }),
      )
      const ref = matches.match(/\[ref=(e\d+)\]/)?.[1]
      if (!ref) throw new Error('missing change button')
      const result = await ctx.mcp.callTool('act', {
        page,
        kind: 'click',
        ref,
        diff: 64,
      })
      const output = outputOf(result)
      expect(result.structuredContent?.contexts).toEqual(
        expect.arrayContaining([
          expect.objectContaining({
            kind: 'diff',
            data: expect.objectContaining({ writtenToFile: true }),
          }),
        ]),
      )
      expect((await bytesOf(ctx, output.id, 'diff.md')).toString()).toContain(
        'Updated fixture button 1799',
      )
      const primary = JSON.parse(
        (await bytesOf(ctx, output.id, 'result.json')).toString(),
      )
      expect(primary.structuredContent.kind).toBe('click')
    },
  },
  {
    name: 'outputs: stateless requests recover the same output through the logical session',
    async run(ctx) {
      const produced = await statelessCall(ctx, 'run', {
        code: "return 'saved value '.repeat(3000)",
      })
      const output = outputOf(produced)
      const session = produced._meta?.['com.browseros.neo/session']
      if (typeof session !== 'string')
        throw new Error('missing logical session handle')
      const recovered = await statelessCall(ctx, 'read_output', {
        session,
        outputId: output.id,
        part: 'result.json',
        limit: 512,
      })
      expectOk(recovered)
      expect(recovered.structuredContent?.text).toContain('saved value ')
      expectError(
        await statelessCall(ctx, 'read_output', { outputId: output.id }),
      )
    },
  },
  {
    name: 'outputs: oversized evaluate preserves the original typed JSON value',
    async run(ctx) {
      const page = await ctx.openPage(ctx.fixture('/media.html'))
      const result = await ctx.mcp.callTool('evaluate_in_page', {
        page,
        code: "return {count:7, text:'line\\n🦀 '.repeat(2000)}",
      })
      const output = outputOf(result)
      const full = JSON.parse(
        (await bytesOf(ctx, output.id, 'result.json')).toString(),
      ).structuredContent
      expect(full.value).toEqual({ count: 7, text: 'line\n🦀 '.repeat(2000) })
    },
  },
  {
    name: 'outputs: oversized run preserves JSON values and logs over paged reads',
    async run(ctx) {
      const result = await ctx.mcp.callTool('run', {
        code: "console.log('saved log'); return {count:7, text:'hello 🦀 '.repeat(5000)}",
      })
      const output = outputOf(result)
      expect(result.structuredContent).not.toHaveProperty('value')
      const full = JSON.parse(
        (await bytesOf(ctx, output.id, 'result.json')).toString(),
      ).structuredContent
      expect(full.value).toEqual({ count: 7, text: 'hello 🦀 '.repeat(5000) })
      expect(full.logs).toEqual(['saved log'])
      const other = await ctx.openSession('other-output-owner')
      expectError(await other.callTool('read_output', { outputId: output.id }))
    },
  },
  {
    name: 'outputs: nested screenshot and PDF share one ID and retain exact bytes',
    async run(ctx) {
      const page = await ctx.openPage(ctx.fixture('/media.html'))
      const result = await ctx.mcp.callTool('run', {
        code: `const p=browser.page(${page}); await p.screenshot({format:'png'}); await p.pdf(); return {done:true}`,
      })
      const output = outputOf(result)
      const image = imageOf(result)
      if (!image) throw new Error('run lost its nested screenshot image')
      expect(await bytesOf(ctx, output.id, 'screenshot.png')).toEqual(
        Buffer.from(image.data, 'base64'),
      )
      const retrieved = imageOf(
        await ctx.mcp.callTool('read_output', {
          outputId: output.id,
          part: 'screenshot.png',
        }),
      )
      expect(retrieved?.data).toBe(image.data)
      const pdf = await bytesOf(ctx, output.id, 'pdf.pdf')
      expect(pdf.subarray(0, 5).toString()).toBe('%PDF-')
      expect(pdf.length).toBe(
        output.parts.find((part) => part.name === 'pdf.pdf')?.bytes,
      )
      expect(result.structuredContent?.value).toEqual({ done: true })
    },
  },
  {
    name: 'outputs: automatic navigate snapshot retains its metadata and saved part',
    async run(ctx) {
      const page = await ctx.openPage('about:blank')
      const result = await ctx.mcp.callTool('navigate', {
        page,
        url: ctx.fixture('/output-large.html'),
      })
      const output = outputOf(result)
      expect(result.structuredContent?.contexts).toEqual(
        expect.arrayContaining([
          expect.objectContaining({
            page,
            kind: 'snapshot',
            data: expect.objectContaining({ writtenToFile: true }),
          }),
        ]),
      )
      const full = (await bytesOf(ctx, output.id, 'snapshot.md')).toString()
      expect(full).toContain('Output fixture button 1799')
      const snapshotCount = output.parts.filter(
        (part) => part.mimeType === 'text/markdown',
      ).length
      expect(snapshotCount).toBe(1)
      expectError(
        await ctx.mcp.callTool('read_output', {
          outputId: output.id,
          part: '../../profile',
        }),
      )
    },
  },
]
