/**
 * The proxy against a real Neo SDK image: what the agent receives through the viewer is exactly
 * what Neo sent. A raw TCP tap between the viewer and the image records Neo's bytes as they left
 * it; each reply the agent read through the viewer must equal them byte for byte. The same calls
 * made directly must give the same reply too, apart from the run's id.
 *
 * Start an image as its README shows (`NEO_MCP_REMOTE=true`, port published on 127.0.0.1), then:
 *   NEO_VIEWER_REAL_UPSTREAM=http://127.0.0.1:9010/mcp bun run test:real-image
 * Without the variable the test is skipped.
 */

import { afterAll, beforeAll, describe, expect, test } from 'bun:test'
import { mkdtemp, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { type RunningViewer, startViewer } from '../server/viewer-server'

const upstream = process.env.NEO_VIEWER_REAL_UPSTREAM
const describeReal = upstream ? describe : describe.skip

/**
 * A `run` call as a client of each MCP revision sends it: Claude Code 2.1's 2026-07-28 shape (no
 * session, clientInfo in every `_meta`), or a 2025-06-18 client's (after its `initialize`).
 */
function runCall(
  id: number,
  code: string,
  revision: '2026-07-28' | '2025-06-18',
) {
  const modern = revision === '2026-07-28'
  const meta = modern
    ? {
        _meta: {
          'io.modelcontextprotocol/protocolVersion': revision,
          'io.modelcontextprotocol/clientInfo': {
            name: 'claude-code',
            version: '2.1.322',
          },
          'io.modelcontextprotocol/clientCapabilities': {},
        },
      }
    : {}
  return {
    headers: {
      'content-type': 'application/json',
      accept: 'application/json, text/event-stream',
      'mcp-protocol-version': revision,
      ...(modern ? { 'mcp-method': 'tools/call', 'mcp-name': 'run' } : {}),
    },
    body: JSON.stringify({
      jsonrpc: '2.0',
      id,
      method: 'tools/call',
      params: { name: 'run', arguments: { code }, ...meta },
    }),
  }
}

/** Every byte each way on every connection through it. */
interface Tap {
  url: string
  fromNeo: () => Uint8Array
  toNeo: () => Uint8Array
  stop: () => void
}

function startTap(target: URL): Tap {
  const fromNeo: Uint8Array[] = []
  const toNeo: Uint8Array[] = []
  type Side = {
    upstream?: Awaited<ReturnType<typeof Bun.connect>>
    pending: Uint8Array[]
  }
  const listener = Bun.listen<Side>({
    hostname: '127.0.0.1',
    port: 0,
    socket: {
      open(client) {
        client.data = { pending: [] }
        void Bun.connect({
          hostname: target.hostname,
          port: Number(target.port),
          socket: {
            data(_, chunk) {
              fromNeo.push(new Uint8Array(chunk))
              client.write(chunk)
            },
            close() {
              client.end()
            },
          },
        }).then((socket) => {
          client.data.upstream = socket
          for (const chunk of client.data.pending) socket.write(chunk)
          client.data.pending = []
        })
      },
      data(client, chunk) {
        const copy = new Uint8Array(chunk)
        toNeo.push(copy)
        if (client.data.upstream) client.data.upstream.write(copy)
        else client.data.pending.push(copy)
      },
      close(client) {
        client.data.upstream?.end()
      },
    },
  })
  return {
    url: `http://127.0.0.1:${listener.port}${target.pathname}`,
    fromNeo: () => Buffer.concat(fromNeo),
    toNeo: () => Buffer.concat(toNeo),
    stop: () => listener.stop(true),
  }
}

/** The bodies of every HTTP/1.1 message in `bytes` (sized or chunked), in order. */
function httpBodies(bytes: Uint8Array): Buffer[] {
  const data = Buffer.from(bytes)
  const bodies: Buffer[] = []
  let at = 0
  while (at < data.length) {
    const headEnd = data.indexOf('\r\n\r\n', at)
    if (headEnd === -1) break
    const head = data.subarray(at, headEnd).toString('latin1').toLowerCase()
    at = headEnd + 4
    const length = /\r\ncontent-length: *(\d+)/.exec(head)
    if (/\r\ntransfer-encoding: *chunked/.test(head)) {
      const parts: Buffer[] = []
      for (;;) {
        const lineEnd = data.indexOf('\r\n', at)
        const size = Number.parseInt(
          data.subarray(at, lineEnd).toString('latin1'),
          16,
        )
        at = lineEnd + 2
        if (size === 0) {
          at += 2
          break
        }
        parts.push(data.subarray(at, at + size))
        at += size + 2
      }
      bodies.push(Buffer.concat(parts))
    } else {
      const size = length ? Number(length[1]) : 0
      bodies.push(data.subarray(at, at + size))
      at += size
    }
  }
  return bodies
}

const withoutRunIds = (text: string) =>
  text.replace(/"id":"[0-9a-f]{8}-[0-9a-f-]{27}"/g, '"id":"<run>"')

describeReal('through a real Neo image', () => {
  let tap: Tap
  let viewer: RunningViewer
  let dataDir: string

  beforeAll(async () => {
    tap = startTap(new URL(upstream as string))
    dataDir = await mkdtemp(join(tmpdir(), 'neo-viewer-real-'))
    viewer = await startViewer({ port: 0, upstream: new URL(tap.url), dataDir })
  })

  afterAll(async () => {
    await viewer?.stop()
    tap?.stop()
    if (dataDir) await rm(dataDir, { recursive: true, force: true })
  })

  // Neo streams its 5 s keep-alive to 2025-06-18 clients only; a 2026-07-28 reply is one event.
  const cases: Array<[string, string, '2026-07-28' | '2025-06-18']> = [
    ['a plain value', 'return 6 * 7', '2026-07-28'],
    ['an error', 'throw new Error("boom")', '2026-07-28'],
    [
      'a reply with console lines',
      'console.log("hi"); console.warn("careful"); return 7',
      '2026-07-28',
    ],
    [
      'a large reply with a blob marker',
      'return { snapshot: Buffer.from("x".repeat(300000)) }',
      '2026-07-28',
    ],
    [
      'a streamed reply (a keep-alive, then the result)',
      'await page.waitForTimeout(6000); return "late"',
      '2025-06-18',
    ],
  ]

  for (const [name, code, revision] of cases) {
    test(name, async () => {
      const call = runCall(1, code, revision)
      const before = {
        fromNeo: tap.fromNeo().length,
        toNeo: tap.toNeo().length,
      }
      const startedAt = Date.now()
      const response = await fetch(viewer.mcpUrl, {
        method: 'POST',
        headers: call.headers,
        body: call.body,
      })
      const reader = response.body?.getReader()
      const chunks: Uint8Array[] = []
      let firstAt = 0
      for (;;) {
        const chunk = await reader?.read()
        if (!chunk || chunk.done) break
        if (chunks.length === 0) firstAt = Date.now()
        chunks.push(chunk.value)
      }
      const endedAt = Date.now()
      const received = Buffer.concat(chunks)
      await Bun.sleep(50)

      // 1. What the agent got is what Neo sent, byte for byte.
      const sent = httpBodies(tap.fromNeo().subarray(before.fromNeo))
      expect(sent).toHaveLength(1)
      expect(received.equals(sent[0] as Buffer)).toBe(true)

      // 2. Neo got the agent's call exactly as sent: the viewer adds nothing.
      const forwarded =
        httpBodies(tap.toNeo().subarray(before.toNeo))[0]?.toString('utf8') ??
        ''
      expect(forwarded).toBe(call.body)

      // 3. The same call made directly answers the same, apart from the run's id.
      const direct = await (
        await fetch(upstream as string, {
          method: 'POST',
          headers: call.headers,
          body: call.body,
        })
      ).text()
      expect(withoutRunIds(received.toString('utf8'))).toBe(
        withoutRunIds(direct),
      )

      if (name.startsWith('a streamed reply')) {
        expect(received.toString('utf8').startsWith(':\n\n')).toBe(true)
        // The keep-alive reached the agent when Neo sent it, not with the result.
        expect(endedAt - firstAt).toBeGreaterThan(500)
        expect(firstAt - startedAt).toBeGreaterThan(4000)
      }
      if (name.startsWith('a large reply'))
        expect(received.toString('utf8')).toContain(
          '{"$neo":"blob","id":"b0","size":300000}',
        )
    }, 30_000)
  }

  test('the log holds those runs in one list, in order, with replies as the agent read them', async () => {
    const runs = viewer.store.list()
    expect(runs.map((run) => run.status)).toEqual([
      'ok',
      'error',
      'ok',
      'ok',
      'ok',
    ])
    // The legacy call names no client, so it is labelled from this test's User-Agent.
    expect(runs.map((run) => run.client?.name)).toEqual([
      'claude-code',
      'claude-code',
      'claude-code',
      'claude-code',
      'Bun',
    ])
    expect(runs[2]?.replyPreview).toBe(
      '### Result\n7\n### Console\nhi\n[warn] careful',
    )
  })
})
