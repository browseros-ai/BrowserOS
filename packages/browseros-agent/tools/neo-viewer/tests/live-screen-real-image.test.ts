/**
 * The live screen against a real Neo SDK image, without a browser: an RFB client speaks through
 * the viewer's relay the way noVNC does, while an "agent" sends `run` calls through the viewer's
 * MCP proxy. It proves view and follow, control (a click and typing reach the page), take over
 * (the agent's run on that tab fails with `owner_has_control`) and give back (Neo closes the
 * owner's display with 4002 and the agent runs again). The agent names no task, so the screen
 * follows its tab; then one run names its task, and the screen follows that task.
 *
 * Start an image with its host API on TCP (`CLAW_HOST_TCP=0.0.0.0:9300`, a token file in
 * `CLAW_HOST_TOKEN_FILE`, both ports published on 127.0.0.1), then:
 *   NEO_VIEWER_REAL_UPSTREAM=http://127.0.0.1:9010/mcp \
 *   NEO_VIEWER_REAL_HOST=http://127.0.0.1:9300 NEO_VIEWER_REAL_HOST_TOKEN_FILE=… \
 *   bun run test:real-image
 * Without the variables the test is skipped.
 */

import { afterAll, beforeAll, describe, expect, test } from 'bun:test'
import { mkdtemp, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import type { LiveHost } from '../server/live-screen/live-host'
import { type RunningViewer, startViewer } from '../server/viewer-server'
import {
  displayClose,
  type LiveStatus,
  liveRoutes,
  type TakeoverResponse,
} from '../shared/live-screen'

const upstream = process.env.NEO_VIEWER_REAL_UPSTREAM
const hostUrl = process.env.NEO_VIEWER_REAL_HOST
const tokenFile = process.env.NEO_VIEWER_REAL_HOST_TOKEN_FILE
const describeReal = upstream && hostUrl && tokenFile ? describe : describe.skip

/** A full-window text area: a mouse press counts in the title, typing fills it. */
const TEST_PAGE = `<title>clicks 0</title><textarea id="t" style="position:fixed;inset:0;width:100vw;height:100vh;font-size:40px" onmousedown="window.clicks=(window.clicks||0)+1;document.title='clicks '+window.clicks"></textarea>`
const READ_PAGE =
  "return { title: await page.title(), value: await page.evaluate(() => document.getElementById('t').value) }"

/** An RFB 3.8 client over the viewer's relay, as noVNC speaks it (security None, shared). */
class RfbClient {
  private buffer = new Uint8Array(0)
  private wake: (() => void) | null = null
  readonly closed: Promise<{ code: number; reason: string }>

  private constructor(private readonly socket: WebSocket) {
    socket.binaryType = 'arraybuffer'
    socket.addEventListener('message', (event) => {
      const chunk = new Uint8Array(event.data as ArrayBuffer)
      const merged = new Uint8Array(this.buffer.length + chunk.length)
      merged.set(this.buffer)
      merged.set(chunk, this.buffer.length)
      this.buffer = merged
      this.wake?.()
    })
    this.closed = new Promise((resolve) =>
      socket.addEventListener('close', (event) => {
        resolve({ code: event.code, reason: event.reason })
        this.wake?.()
      }),
    )
  }

  static open(viewer: RunningViewer, query: string): RfbClient {
    const url = `${viewer.url.replace('http', 'ws')}${liveRoutes.display}?${query}`
    const socket = new (
      WebSocket as unknown as new (
        url: string,
        options: { headers: Record<string, string> },
      ) => WebSocket
    )(url, { headers: { Origin: viewer.url } })
    return new RfbClient(socket)
  }

  private async read(length: number): Promise<Uint8Array> {
    while (this.buffer.length < length) {
      if (this.socket.readyState >= WebSocket.CLOSING)
        throw new Error(`display closed: ${JSON.stringify(await this.closed)}`)
      await new Promise<void>((resolve) => {
        this.wake = resolve
      })
    }
    const out = this.buffer.slice(0, length)
    this.buffer = this.buffer.slice(length)
    return out
  }

  async handshake(): Promise<{ width: number; height: number }> {
    expect(new TextDecoder().decode(await this.read(12))).toBe('RFB 003.008\n')
    this.socket.send(new TextEncoder().encode('RFB 003.008\n'))
    const [count = 0] = await this.read(1)
    expect([...(await this.read(count))]).toContain(1)
    this.socket.send(Uint8Array.of(1))
    expect(new DataView((await this.read(4)).buffer).getUint32(0)).toBe(0)
    this.socket.send(Uint8Array.of(1))
    const init = new DataView((await this.read(24)).buffer)
    await this.read(init.getUint32(20))
    return { width: init.getUint16(0), height: init.getUint16(2) }
  }

  click(x: number, y: number): void {
    for (const mask of [1, 0]) {
      const message = new DataView(new ArrayBuffer(6))
      message.setUint8(0, 5)
      message.setUint8(1, mask)
      message.setUint16(2, x)
      message.setUint16(4, y)
      this.socket.send(message.buffer)
    }
  }

  type(text: string): void {
    for (const char of text)
      for (const down of [1, 0]) {
        const message = new DataView(new ArrayBuffer(8))
        message.setUint8(0, 4)
        message.setUint8(1, down)
        message.setUint32(4, char.charCodeAt(0))
        this.socket.send(message.buffer)
      }
  }

  close(): void {
    this.socket.close(1000)
  }
}

describeReal('the live screen on a real image', () => {
  let viewer: RunningViewer
  let dataDir: string
  const task = `live-real-${Date.now()}`

  /** A `run` through the viewer's MCP proxy; with `taskId`, the agent names its own task. */
  async function run(
    code: string,
    taskId?: string,
  ): Promise<{ text: string; isError: boolean }> {
    const response = await fetch(viewer.mcpUrl, {
      method: 'POST',
      headers: {
        'content-type': 'application/json',
        accept: 'application/json, text/event-stream',
        'mcp-protocol-version': '2025-06-18',
      },
      body: JSON.stringify({
        jsonrpc: '2.0',
        id: 1,
        method: 'tools/call',
        params: {
          name: 'run',
          arguments: { code },
          ...(taskId ? { _meta: { 'com.browseros.neo/run': { taskId } } } : {}),
        },
      }),
    })
    const raw = await response.text()
    const data = raw
      .split('\n')
      .filter((line) => line.startsWith('data: '))
      .pop()
    const body = JSON.parse(data ? data.slice(6) : raw) as {
      result: { content: Array<{ text: string }>; isError?: boolean }
    }
    return {
      text: body.result.content.map((block) => block.text).join('\n'),
      isError: body.result.isError === true,
    }
  }

  async function liveStatus(): Promise<LiveStatus> {
    return (await (
      await fetch(`${viewer.url}${liveRoutes.status}`)
    ).json()) as LiveStatus
  }

  function post(path: string, body: unknown): Promise<Response> {
    return fetch(`${viewer.url}${path}`, {
      method: 'POST',
      headers: { 'content-type': 'application/json', origin: viewer.url },
      body: JSON.stringify(body),
    })
  }

  /** The middle of the page's content on Neo's screen: below the tab strip and address bar. */
  async function pageCenter(): Promise<{ x: number; y: number }> {
    const { text } = await run(
      'return await page.evaluate(() => ({ x: screenX, y: screenY, outerWidth, outerHeight, innerHeight }))',
    )
    const box = JSON.parse(text) as Record<string, number>
    const chrome = (box.outerHeight ?? 0) - (box.innerHeight ?? 0)
    return {
      x: Math.round((box.x ?? 0) + (box.outerWidth ?? 0) / 2),
      y: Math.round((box.y ?? 0) + chrome + (box.innerHeight ?? 0) / 2),
    }
  }

  beforeAll(async () => {
    const token = (await Bun.file(tokenFile as string).text()).trim()
    const base = hostUrl as string
    const liveHost: LiveHost = {
      fetch: (path, init = {}) => {
        const headers = new Headers(init.headers)
        headers.set('authorization', `Bearer ${token}`)
        return fetch(`${base}${path}`, { ...init, headers })
      },
      socket: (path) => ({
        url: `${base.replace(/^http/, 'ws')}${path}`,
        headers: { Authorization: `Bearer ${token}` },
      }),
    }
    dataDir = await mkdtemp(join(tmpdir(), 'neo-viewer-live-real-'))
    viewer = await startViewer(
      { port: 0, upstream: new URL(upstream as string), dataDir },
      { liveHost },
    )
  })

  afterAll(async () => {
    await viewer?.stop()
    await rm(dataDir, { recursive: true, force: true })
  })

  test('view follows the task, control clicks and types, take over holds the agent', async () => {
    expect(
      (
        await run(
          `await page.setContent(${JSON.stringify(TEST_PAGE)}); return 'ready'`,
        )
      ).text,
    ).toBe('"ready"')
    const status = await liveStatus()
    expect(status.host).toEqual({ state: 'up' })
    // The agent named no task: the screen follows the tab its latest run acted on.
    expect(status.agent?.tab.title).toBe('clicks 0')
    expect(status.agent?.taskId).toBeNull()
    const agentTab = status.agent?.tab.tabId as number

    // Follow agent, by tab.
    const view = RfbClient.open(viewer, `mode=view&followTab=${agentTab}`)
    const screen = await view.handshake()
    expect(screen.width).toBeGreaterThan(0)
    view.close()

    // Control: a click and typing reach the page.
    const center = await pageCenter()
    const control = RfbClient.open(viewer, 'mode=control')
    await control.handshake()
    control.click(center.x, center.y)
    await Bun.sleep(300)
    control.type('neo')
    await Bun.sleep(700)
    control.close()
    expect(JSON.parse((await run(READ_PAGE)).text)).toEqual({
      title: 'clicks 1',
      value: 'neo',
    })

    // Take over: the agent's calls on the tab fail; the owner's display still works.
    // As the panel does: Neo's own screen size, so Neo fits and brings the tab forward.
    const taken = await post(liveRoutes.takeovers, {
      tabId: agentTab,
      viewer: screen,
    })
    expect(taken.status).toBe(200)
    const { takeover } = (await taken.json()) as TakeoverResponse
    expect(takeover.tabId).toBe(agentTab)
    const held = await run(READ_PAGE)
    expect(held.isError).toBe(true)
    expect(held.text).toContain('owner_has_control')
    const owner = RfbClient.open(viewer, `mode=control&release=${takeover.id}`)
    await owner.handshake()
    owner.click(center.x, center.y)
    await Bun.sleep(500)

    // Give back: Neo closes the owner's display with 4002 and the agent runs again.
    expect((await post(liveRoutes.release(takeover.id), {})).status).toBe(200)
    expect((await owner.closed).code).toBe(displayClose.released)
    expect(JSON.parse((await run(READ_PAGE)).text)).toEqual({
      title: 'clicks 2',
      value: 'neo',
    })

    // A run that names its own task: the screen follows that task (its tab, and wherever it goes).
    expect(
      (
        await run(
          "await page.setContent('<title>tasked</title>'); return 'ok'",
          task,
        )
      ).text,
    ).toBe('"ok"')
    const tasked = await liveStatus()
    expect(tasked.agent?.taskId).toBe(task)
    expect(tasked.agent?.tab.title).toBe('tasked')
    const byTask = RfbClient.open(viewer, `mode=view&followTask=${task}`)
    expect((await byTask.handshake()).width).toBe(screen.width)
    byTask.close()
  })
})
