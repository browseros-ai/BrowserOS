/**
 * CLAW_MCP_CAPTURE_DIR mode: alongside a suite run, dump the raw CDP
 * payloads the snapshot pipeline consumes — `Accessibility.getFullAXTree`
 * per fixture page (the exact `{"nodes":[…]}` shape of
 * `crates/browseros-core/tests/data/get-full-ax-tree.json`), plus
 * `Page.getFrameTree` and a `DOM.describeNode` sample — so the
 * browseros-core serde fixtures can be refreshed from a real browser:
 *
 *   CLAW_MCP_CAPTURE_DIR=crates/browseros-core/tests/data/captured \
 *     BROWSEROS_BINARY=… bun contracts/claw-mcp/tests/run.ts --smoke
 *
 * Talks CDP directly (browser websocket + flattened target sessions);
 * deliberately independent of either claw server so a capture never
 * depends on the code under test.
 */

import { mkdir, writeFile } from 'node:fs/promises'
import { join } from 'node:path'
import type { FixtureServer } from '../fixtures/server'
import { listFixturePages } from '../fixtures/server'
import { waitUntil } from './helpers'

/** Excluded from capture: their generated 20k-node/oversized trees would bloat committed fixtures. */
const CAPTURE_SKIP = new Set(['dynamic.html', 'media.html'])

interface CdpMessage {
  id?: number
  method?: string
  params?: Record<string, unknown>
  sessionId?: string
  result?: Record<string, unknown>
  error?: { message: string }
}

class CdpConnection {
  #ws: WebSocket
  #nextId = 1
  #pending = new Map<
    number,
    {
      resolve: (value: Record<string, unknown>) => void
      reject: (error: Error) => void
    }
  >()
  #eventWaiters: Array<{
    method: string
    sessionId?: string
    resolve: () => void
  }> = []

  private constructor(ws: WebSocket) {
    this.#ws = ws
    ws.addEventListener('message', (event) => {
      const message = JSON.parse(String(event.data)) as CdpMessage
      if (message.id !== undefined) {
        const pending = this.#pending.get(message.id)
        if (pending) {
          this.#pending.delete(message.id)
          if (message.error) pending.reject(new Error(message.error.message))
          else pending.resolve(message.result ?? {})
        }
        return
      }
      if (message.method) {
        this.#eventWaiters = this.#eventWaiters.filter((waiter) => {
          if (
            waiter.method === message.method &&
            (waiter.sessionId === undefined ||
              waiter.sessionId === message.sessionId)
          ) {
            waiter.resolve()
            return false
          }
          return true
        })
      }
    })
  }

  static async connect(cdpPort: number): Promise<CdpConnection> {
    const version = (await (
      await fetch(`http://127.0.0.1:${cdpPort}/json/version`)
    ).json()) as { webSocketDebuggerUrl: string }
    const ws = new WebSocket(version.webSocketDebuggerUrl)
    await new Promise<void>((resolve, reject) => {
      ws.addEventListener('open', () => resolve(), { once: true })
      ws.addEventListener(
        'error',
        () => reject(new Error('CDP socket failed')),
        {
          once: true,
        },
      )
    })
    return new CdpConnection(ws)
  }

  async send(
    method: string,
    params: Record<string, unknown> = {},
    sessionId?: string,
  ): Promise<Record<string, unknown>> {
    const id = this.#nextId
    this.#nextId += 1
    const payload: CdpMessage = { id, method, params }
    if (sessionId) payload.sessionId = sessionId
    this.#ws.send(JSON.stringify(payload))
    return await new Promise((resolve, reject) => {
      this.#pending.set(id, { resolve, reject })
      setTimeout(() => {
        if (this.#pending.delete(id)) {
          reject(new Error(`CDP ${method} timed out`))
        }
      }, 15_000)
    })
  }

  waitForEvent(method: string, sessionId?: string): Promise<void> {
    return new Promise((resolve) => {
      this.#eventWaiters.push({ method, sessionId, resolve })
    })
  }

  close(): void {
    this.#ws.close()
  }
}

async function capturePage(
  cdp: CdpConnection,
  url: string,
  outDir: string,
): Promise<void> {
  const { targetId } = (await cdp.send('Target.createTarget', {
    url: 'about:blank',
  })) as { targetId: string }
  const { sessionId } = (await cdp.send('Target.attachToTarget', {
    targetId,
    flatten: true,
  })) as { sessionId: string }
  try {
    await cdp.send('Page.enable', {}, sessionId)
    const loaded = cdp.waitForEvent('Page.loadEventFired', sessionId)
    await cdp.send('Page.navigate', { url }, sessionId)
    await loaded
    await cdp.send('Accessibility.enable', {}, sessionId)

    let axTree: Record<string, unknown> = {}
    await waitUntil(
      async () => {
        axTree = await cdp.send('Accessibility.getFullAXTree', {}, sessionId)
        const nodes = axTree.nodes as unknown[] | undefined
        return Array.isArray(nodes) && nodes.length > 3
      },
      `accessibility tree of ${url}`,
      { timeoutMs: 10_000, intervalMs: 250 },
    )
    const frameTree = await cdp.send('Page.getFrameTree', {}, sessionId)
    const document = (await cdp.send(
      'DOM.getDocument',
      { depth: 1 },
      sessionId,
    )) as { root: { nodeId: number } }
    const describeNode = await cdp.send(
      'DOM.describeNode',
      { nodeId: document.root.nodeId, depth: 2 },
      sessionId,
    )

    await mkdir(outDir, { recursive: true })
    const dump = (name: string, value: Record<string, unknown>) =>
      writeFile(join(outDir, name), `${JSON.stringify(value, null, 2)}\n`)
    await dump('get-full-ax-tree.json', axTree)
    await dump('get-frame-tree.json', frameTree)
    await dump('describe-node.json', describeNode)
  } finally {
    await cdp.send('Target.closeTarget', { targetId }).catch(() => {})
  }
}

export async function runCaptureMode(
  cdpPort: number,
  fixtures: FixtureServer,
  captureDir: string,
): Promise<void> {
  const cdp = await CdpConnection.connect(cdpPort)
  try {
    for (const page of await listFixturePages()) {
      if (CAPTURE_SKIP.has(page)) continue
      const slug = page.replace(/\.html$/, '')
      await capturePage(cdp, fixtures.url(`/${page}`), join(captureDir, slug))
      console.log(`captured CDP payloads for ${page}`)
    }
  } finally {
    cdp.close()
  }
}
