import { afterEach, beforeEach, describe, expect, it, mock } from 'bun:test'
import { parseHTML } from 'linkedom'
import { act, type ReactNode } from 'react'
import type { Root } from 'react-dom/client'
import { renderToStaticMarkup } from 'react-dom/server'
import * as auditHooks from '@/modules/api/audit.hooks'

mock.module('@/modules/api/audit.hooks', () => ({
  ...auditHooks,
  useApiBaseUrl: () => 'http://127.0.0.1:9210',
}))

const { MiniScreencast } = await import('./MiniScreencast')

/** A socket the test opens, feeds and drops, standing in for claw-server. */
class FakeWebSocket {
  static readonly CONNECTING = 0
  static readonly OPEN = 1
  static readonly CLOSED = 3
  static instances: FakeWebSocket[] = []
  readyState = FakeWebSocket.CONNECTING
  binaryType = 'blob'
  sent: unknown[] = []
  closed = false
  onopen: (() => void) | null = null
  onmessage: ((event: { data: ArrayBuffer }) => void) | null = null
  onclose: (() => void) | null = null
  onerror: (() => void) | null = null
  constructor(readonly url: string) {
    FakeWebSocket.instances.push(this)
  }
  send(data: string) {
    this.sent.push(JSON.parse(data))
  }
  close() {
    this.readyState = FakeWebSocket.CLOSED
    this.closed = true
  }
}

const names = [
  'window',
  'document',
  'navigator',
  'HTMLElement',
  'Node',
  'Event',
  'WebSocket',
  'setTimeout',
  'clearTimeout',
  'IS_REACT_ACT_ENVIRONMENT',
]
const descriptors = new Map(
  names.map((name) => [
    name,
    Object.getOwnPropertyDescriptor(globalThis, name),
  ]),
)
const createObjectURL = URL.createObjectURL
const revokeObjectURL = URL.revokeObjectURL
let root: Root
let container: HTMLElement
let visibility: DocumentVisibilityState
let urls: Set<string>
let timers: Map<number, { at: number; callback: () => void }>
let now: number

beforeEach(async () => {
  FakeWebSocket.instances = []
  urls = new Set()
  timers = new Map()
  now = 0
  let nextTimer = 0
  let nextUrl = 0
  visibility = 'visible'
  const dom = parseHTML('<html><body><div id="root"></div></body></html>')
  const globals = {
    window: dom.window,
    document: dom.document,
    navigator: dom.window.navigator,
    HTMLElement: dom.window.HTMLElement,
    Node: dom.window.Node,
    Event: dom.window.Event,
    WebSocket: FakeWebSocket,
    IS_REACT_ACT_ENVIRONMENT: true,
    setTimeout: (callback: () => void, ms: number) => {
      const id = ++nextTimer
      timers.set(id, { at: now + ms, callback })
      return id
    },
    clearTimeout: (id: number) => timers.delete(id),
  }
  for (const [name, value] of Object.entries(globals)) {
    Object.defineProperty(globalThis, name, {
      configurable: true,
      writable: true,
      value,
    })
  }
  Object.defineProperty(document, 'visibilityState', {
    configurable: true,
    get: () => visibility,
  })
  URL.createObjectURL = () => {
    const url = `blob:frame-${++nextUrl}`
    urls.add(url)
    return url
  }
  URL.revokeObjectURL = (url) => {
    urls.delete(url)
  }
  const element = document.getElementById('root')
  if (!element) throw new Error('Missing fixture container')
  container = element
  const { createRoot } = await import('react-dom/client')
  root = createRoot(container)
})

afterEach(async () => {
  await act(async () => root.unmount())
  expect(urls.size).toBe(0)
  expect(timers.size).toBe(0)
  expect(FakeWebSocket.instances.every((socket) => socket.closed)).toBe(true)
  for (const [name, descriptor] of descriptors) {
    if (descriptor) Object.defineProperty(globalThis, name, descriptor)
    else Reflect.deleteProperty(globalThis, name)
  }
  URL.createObjectURL = createObjectURL
  URL.revokeObjectURL = revokeObjectURL
})

async function render(node: ReactNode) {
  await act(async () => root.render(node))
}

const card = (sessionId: string, browserTabId?: number) => (
  <MiniScreencast
    key={sessionId}
    site={`${sessionId}.example`}
    sessionId={sessionId}
    browserTabId={browserTabId}
    live
  />
)

async function advance(ms: number) {
  const end = now + ms
  while (true) {
    const next = [...timers].sort((a, b) => a[1].at - b[1].at)[0]
    if (!next || next[1].at > end) break
    now = next[1].at
    timers.delete(next[0])
    await act(async () => {
      next[1].callback()
    })
  }
  now = end
}

function socket(index = -1): FakeWebSocket {
  const found = FakeWebSocket.instances.at(index)
  if (!found) throw new Error('No socket was opened')
  return found
}

async function open(target = socket()) {
  await act(async () => {
    target.readyState = FakeWebSocket.OPEN
    target.onopen?.()
  })
}

async function drop(target = socket()) {
  await act(async () => {
    target.readyState = FakeWebSocket.CLOSED
    target.closed = true
    target.onclose?.()
  })
}

/** One frame as claw-server sends it: header length, JSON header, JPEG. */
async function frame(
  watch: { sessionId: string; browserTabId?: number },
  jpeg = 'jpeg',
  target = socket(),
) {
  const header = new TextEncoder().encode(
    JSON.stringify({
      ...watch,
      tabId: 7,
      url: 'https://a.test',
      capturedAt: 1,
    }),
  )
  const body = new TextEncoder().encode(jpeg)
  const data = new Uint8Array(4 + header.length + body.length)
  new DataView(data.buffer).setUint32(0, header.length)
  data.set(header, 4)
  data.set(body, 4 + header.length)
  await act(async () => target.onmessage?.({ data: data.buffer }))
}

async function setVisibility(value: DocumentVisibilityState) {
  await act(async () => {
    visibility = value
    document.dispatchEvent(new Event('visibilitychange'))
  })
}

const shown = (sessionId: string) =>
  container
    .querySelector(`[data-live-session="${sessionId}"] img`)
    ?.getAttribute('src') ?? null

describe('MiniScreencast live frames', () => {
  it('renders a placeholder during SSR without opening a socket', () => {
    expect(
      renderToStaticMarkup(
        <MiniScreencast site="example.com" sessionId="one" />,
      ),
    ).toContain('example.com')
    expect(FakeWebSocket.instances).toHaveLength(0)
  })

  it('shares one socket between cards and sends their watches once it opens', async () => {
    await render([card('one', 101), card('two')])
    expect(FakeWebSocket.instances).toHaveLength(1)
    expect(socket().url).toBe('ws://127.0.0.1:9210/api/v1/screencasts')
    expect(socket().sent).toHaveLength(0)

    await open()
    expect(socket().sent).toEqual([
      {
        watch: [{ sessionId: 'one', browserTabId: 101 }, { sessionId: 'two' }],
      },
    ])
  })

  it('shows each frame on its own card and keeps only the newest image', async () => {
    await render([card('one', 101), card('two')])
    await open()
    expect(shown('one')).toBeNull()

    await frame({ sessionId: 'one', browserTabId: 101 })
    expect(shown('one')).toBe('blob:frame-1')
    expect(shown('two')).toBeNull()
    expect(container.querySelector('.animate-pulse-dot')).not.toBeNull()

    await frame({ sessionId: 'two' })
    await frame({ sessionId: 'one', browserTabId: 101 })
    expect(shown('one')).toBe('blob:frame-3')
    expect(shown('two')).toBe('blob:frame-2')
    expect(urls).toEqual(new Set(['blob:frame-2', 'blob:frame-3']))

    // A frame for something no card watches changes nothing.
    await frame({ sessionId: 'one' })
    expect(urls.size).toBe(2)
  })

  it('closes the socket while the cockpit is hidden and resubscribes when it returns', async () => {
    await render(card('one'))
    await open()
    await frame({ sessionId: 'one' })

    await setVisibility('hidden')
    expect(socket().closed).toBe(true)
    await advance(60_000)
    expect(FakeWebSocket.instances).toHaveLength(1)
    // The card keeps its last frame rather than flashing back to the placeholder.
    expect(shown('one')).toBe('blob:frame-1')

    await setVisibility('visible')
    expect(FakeWebSocket.instances).toHaveLength(2)
    await open()
    expect(socket().sent).toEqual([{ watch: [{ sessionId: 'one' }] }])
  })

  it('reconnects after the socket drops and sends the watches again', async () => {
    await render(card('one'))
    await drop()
    await advance(999)
    expect(FakeWebSocket.instances).toHaveLength(1)
    await advance(1)
    expect(FakeWebSocket.instances).toHaveLength(2)

    // Repeated failures back off.
    await drop()
    await advance(1999)
    expect(FakeWebSocket.instances).toHaveLength(2)
    await advance(1)
    expect(FakeWebSocket.instances).toHaveLength(3)

    await open()
    expect(socket().sent).toEqual([{ watch: [{ sessionId: 'one' }] }])
  })

  it('moves a card to its new tab and releases the old frame', async () => {
    await render(card('one', 101))
    await open()
    await frame({ sessionId: 'one', browserTabId: 101 })

    await render(card('one', 102))
    expect(shown('one')).toBeNull()
    expect(urls.size).toBe(0)
    expect(socket().sent.at(-1)).toEqual({
      watch: [{ sessionId: 'one', browserTabId: 102 }],
    })
    await frame({ sessionId: 'one', browserTabId: 101 })
    expect(shown('one')).toBeNull()
  })

  it('closes the socket when the last card goes', async () => {
    await render([card('one'), card('two')])
    await open()
    await frame({ sessionId: 'one' })

    await render(card('two'))
    expect(socket().closed).toBe(false)
    expect(socket().sent.at(-1)).toEqual({ watch: [{ sessionId: 'two' }] })
    expect(urls.size).toBe(0)

    await render(null)
    expect(socket().closed).toBe(true)
  })
})
