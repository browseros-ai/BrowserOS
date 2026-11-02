/** Saved output contracts cross the real MCP wire; no host filesystem reads are used. */
import { expect } from 'bun:test'
import type { CaseContext, ContractCase } from './cases'
import { expectError, expectOk } from './helpers'
import { imageOf, type McpToolResult, readRpcResponse } from './mcp-client'

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
