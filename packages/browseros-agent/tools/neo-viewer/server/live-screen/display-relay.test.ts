import { afterEach, expect, test } from 'bun:test'
import { mkdtemp, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { displayClose, liveRoutes } from '../../shared/live-screen'
import {
  FAKE_TOKEN,
  type FakeNeoHost,
  startFakeNeoHost,
  unreachableHost,
} from '../test-support/fake-neo-host'
import { type RunningViewer, startViewer } from '../viewer-server'
import { pageCloseFor } from './display-relay'
import type { LiveHost } from './live-host'

const cleanups: Array<() => Promise<unknown>> = []
afterEach(async () => {
  while (cleanups.length > 0) await cleanups.pop()?.()
})

async function viewerWith(liveHost: LiveHost): Promise<RunningViewer> {
  const dataDir = await mkdtemp(join(tmpdir(), 'neo-viewer-live-'))
  cleanups.push(() => rm(dataDir, { recursive: true, force: true }))
  const viewer = await startViewer(
    { port: 0, upstream: new URL('http://127.0.0.1:9/mcp'), dataDir },
    { liveHost },
  )
  cleanups.push(() => viewer.stop())
  return viewer
}

async function fakeNeo(): Promise<FakeNeoHost> {
  const neo = startFakeNeoHost()
  cleanups.push(() => neo.stop())
  return neo
}

/** The page's side: a WebSocket to the viewer's relay, as the panel opens it. */
interface PageSocket {
  socket: WebSocket
  frames: Array<string | Uint8Array>
  opened: Promise<void>
  closed: Promise<{ code: number; reason: string }>
  nextFrame(): Promise<string | Uint8Array>
}

function openPage(
  viewer: RunningViewer,
  query: string,
  origin: string | null = viewer.url,
): PageSocket {
  const url = `${viewer.url.replace('http', 'ws')}${liveRoutes.display}?${query}`
  // Bun's client takes headers; the page's browser sends its own Origin.
  const socket = new (
    WebSocket as unknown as new (
      url: string,
      options: { headers: Record<string, string> },
    ) => WebSocket
  )(url, { headers: origin ? { Origin: origin } : {} })
  socket.binaryType = 'arraybuffer'
  const frames: Array<string | Uint8Array> = []
  const waiting: Array<(frame: string | Uint8Array) => void> = []
  socket.addEventListener('message', (event) => {
    const frame =
      typeof event.data === 'string'
        ? event.data
        : new Uint8Array(event.data as ArrayBuffer)
    frames.push(frame)
    waiting.shift()?.(frame)
  })
  return {
    socket,
    frames,
    opened: new Promise((resolve) =>
      socket.addEventListener('open', () => resolve()),
    ),
    closed: new Promise((resolve) =>
      socket.addEventListener('close', (event) =>
        resolve({ code: event.code, reason: event.reason }),
      ),
    ),
    nextFrame: () =>
      new Promise((resolve) => {
        waiting.push(resolve)
      }),
  }
}

test('frames pass both ways byte for byte, and only Neo sees the token', async () => {
  const neo = await fakeNeo()
  const viewer = await viewerWith(neo.host)
  const page = openPage(viewer, 'mode=view&followTask=task-1')

  const greeting = await page.nextFrame()
  expect(new TextDecoder().decode(greeting as Uint8Array)).toBe('RFB 003.008\n')
  const [display] = neo.displays
  expect(display?.path).toBe('/v1/display?mode=view&followTask=task-1')
  expect(display?.authorization).toBe(`Bearer ${FAKE_TOKEN}`)

  // Every byte value, so nothing is re-encoded on the way.
  const input = Uint8Array.from({ length: 256 }, (_, i) => i)
  page.socket.send(input)
  page.socket.send('a text frame passes as text')
  await Bun.sleep(50)
  expect(display?.received).toEqual([input, 'a text frame passes as text'])

  const screen = Uint8Array.from({ length: 70_000 }, (_, i) => (i * 7) % 256)
  display?.socket.sendBinary(screen)
  expect(await page.nextFrame()).toEqual(screen)

  display?.socket.close(4002, 'released')
  const closed = await page.closed
  expect(closed).toEqual({ code: displayClose.released, reason: 'released' })
  const seen = [...page.frames, closed.reason]
    .map((frame) =>
      typeof frame === 'string' ? frame : new TextDecoder().decode(frame),
    )
    .join('\n')
  expect(seen).not.toContain(FAKE_TOKEN)
})

test('a page that goes away closes Neo’s display with 1000', async () => {
  const neo = await fakeNeo()
  const viewer = await viewerWith(neo.host)
  const page = openPage(viewer, 'mode=control')
  await page.nextFrame()
  const closing = neo.nextDisplayClose()
  page.socket.close(1001, 'tab closed')
  const display = await closing
  expect(display.path).toBe('/v1/display?mode=control')
  expect(display.closedWith?.code).toBe(1000)
})

test('a display Neo refuses closes the page with Neo’s own error', async () => {
  const neo = await fakeNeo()
  neo.respond = (request) =>
    request.path.startsWith('/v1/display?mode=control&release=')
      ? Response.json(
          { code: 'not_found', message: 'no takeover has this id' },
          { status: 404 },
        )
      : undefined
  const viewer = await viewerWith(neo.host)
  const page = openPage(viewer, 'mode=control&release=tk_gone')
  expect(await page.closed).toEqual({
    code: displayClose.displayRefused,
    reason: 'not_found: no takeover has this id',
  })
  expect(neo.displays).toHaveLength(0)
})

test('a good query Neo cannot open is display_unavailable', async () => {
  const neo = await fakeNeo()
  // Neo admits a display after the upgrade check: the upgrade gets 503, a plain GET gets 400.
  neo.respond = (request) =>
    request.path.startsWith('/v1/display')
      ? Response.json(
          { code: 'display_unavailable', message: 'Xvnc is not running' },
          { status: 503 },
        )
      : undefined
  const realFetch = neo.host.fetch
  neo.host.fetch = async (path, init) =>
    path.startsWith('/v1/display')
      ? Response.json(
          { code: 'invalid_request', message: 'Bad Request' },
          { status: 400 },
        )
      : realFetch(path, init)
  const viewer = await viewerWith(neo.host)
  const closed = await openPage(viewer, 'mode=view').closed
  expect(closed.code).toBe(displayClose.displayRefused)
  expect(closed.reason).toStartWith('display_unavailable:')
})

test('no Neo at all is host-unreachable, and so is a host API that is not set up', async () => {
  const viewer = await viewerWith(unreachableHost())
  const closed = await openPage(viewer, 'mode=view').closed
  expect(closed.code).toBe(displayClose.hostUnreachable)
  expect(closed.reason).toStartWith("Neo's host API is not reachable")

  const unset = await viewerWith({
    fetch: () => Promise.reject(new Error('no host API configured')),
    socket: () => {
      throw new Error('no host API configured')
    },
  })
  expect(await openPage(unset, 'mode=view').closed).toEqual({
    code: displayClose.hostUnreachable,
    reason: "Neo's host API is not set up: no host API configured",
  })
})

test('the host API client’s own “no Neo” answers are host-unreachable too', async () => {
  for (const [status, code] of [
    [502, 'host_api_unreachable'],
    [503, 'no_host_api'],
  ] as const) {
    const viewer = await viewerWith({
      fetch: async () =>
        Response.json({ code, message: `no Neo (${code})` }, { status }),
      socket: (path) => unreachableHost().socket(path),
    })
    expect(await openPage(viewer, 'mode=view').closed).toEqual({
      code: displayClose.hostUnreachable,
      reason: `no Neo (${code})`,
    })
  }
})

test('a connection that drops without a close frame is displayLost', async () => {
  const neo = await fakeNeo()
  const viewer = await viewerWith(neo.host)
  const page = openPage(viewer, 'mode=view')
  await page.nextFrame()
  neo.displays[0]?.socket.terminate()
  expect((await page.closed).code).toBe(displayClose.displayLost)
})

test('only the viewer’s own page may open a display, with a query Neo would accept', async () => {
  const neo = await fakeNeo()
  const viewer = await viewerWith(neo.host)
  for (const origin of [null, 'http://evil.example', 'http://127.0.0.1:1']) {
    const page = openPage(viewer, 'mode=view', origin)
    expect((await page.closed).code).toBe(1002)
  }
  const display = `${viewer.url}${liveRoutes.display}`
  const refused = await fetch(`${display}?mode=view`, {
    headers: { origin: 'http://evil.example' },
  })
  expect(refused.status).toBe(403)
  for (const query of [
    'mode=view&release=tk_1',
    'mode=control&followTask=t1',
    'mode=view&followTab=3',
    'mode=bogus',
    `mode=view&followTask=${'x'.repeat(129)}`,
  ]) {
    const answer = await fetch(`${display}?${query}`, {
      headers: { origin: viewer.url },
    })
    expect(answer.status).toBe(400)
  }
  const notUpgrade = await fetch(`${display}?mode=view`, {
    headers: { origin: viewer.url },
  })
  expect(notUpgrade.status).toBe(426)
  expect(neo.displays).toHaveLength(0)
})

test('close codes a server cannot send become ones it can', () => {
  expect(pageCloseFor(1005, '')).toEqual([1000, ''])
  expect(pageCloseFor(1006, '')[0]).toBe(displayClose.displayLost)
  expect(pageCloseFor(1015, '')[0]).toBe(displayClose.displayLost)
  expect(pageCloseFor(1011, 'Xvnc closed')).toEqual([1011, 'Xvnc closed'])
  expect(pageCloseFor(1008, 'paste too big')).toEqual([1008, 'paste too big'])
  expect(pageCloseFor(4002, 'released')).toEqual([4002, 'released'])
  expect(pageCloseFor(1004, '')[0]).toBe(1011)
})
