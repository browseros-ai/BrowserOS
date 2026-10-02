import { afterEach, beforeEach, describe, expect, it, mock } from 'bun:test'
import { parseHTML } from 'linkedom'
import { act } from 'react'
import type { Root } from 'react-dom/client'
import { renderToStaticMarkup } from 'react-dom/server'
import * as auditHooks from '@/modules/api/audit.hooks'

mock.module('@/modules/api/audit.hooks', () => ({
  ...auditHooks,
  useApiBaseUrl: () => 'http://127.0.0.1:9210',
}))

const { MiniScreencast } = await import('./MiniScreencast')

/** Browser boundaries are controllable so slow fetch/decode cannot hide overlap. */
class FakeImage {
  static instances: FakeImage[] = []
  onload: (() => void) | null = null
  onerror: (() => void) | null = null
  src = ''
  constructor() {
    FakeImage.instances.push(this)
  }
}

interface Request {
  url: string
  signal: AbortSignal
  resolve: (response: Response) => void
}

const names = [
  'window',
  'document',
  'navigator',
  'HTMLElement',
  'Node',
  'Event',
  'Image',
  'fetch',
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
const dateNow = Date.now
let root: Root
let container: HTMLElement
let visibility: DocumentVisibilityState
let requests: Request[]
let urls: Set<string>
let timers: Map<number, { at: number; callback: () => void }>
let now: number

beforeEach(async () => {
  FakeImage.instances = []
  requests = []
  urls = new Set()
  timers = new Map()
  now = 100
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
    Image: FakeImage,
    IS_REACT_ACT_ENVIRONMENT: true,
    setTimeout: (callback: () => void, ms: number) => {
      const id = ++nextTimer
      timers.set(id, { at: now + ms, callback })
      return id
    },
    clearTimeout: (id: number) => timers.delete(id),
    fetch: (url: string, init: RequestInit) =>
      new Promise<Response>((resolve, reject) => {
        const signal = init.signal
        if (!signal) throw new Error('Preview requests need cancellation')
        requests.push({ url, signal, resolve })
        signal.addEventListener(
          'abort',
          () => reject(new DOMException('aborted', 'AbortError')),
          { once: true },
        )
      }),
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
  Date.now = () => now
  URL.createObjectURL = () => {
    const url = `blob:preview-${++nextUrl}`
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
  for (const [name, descriptor] of descriptors) {
    if (descriptor) Object.defineProperty(globalThis, name, descriptor)
    else Reflect.deleteProperty(globalThis, name)
  }
  URL.createObjectURL = createObjectURL
  URL.revokeObjectURL = revokeObjectURL
  Date.now = dateNow
})

async function render(sessionId = 'session-1', browserTabId?: number) {
  await act(async () =>
    root.render(
      <MiniScreencast
        site="example.com"
        sessionId={sessionId}
        browserTabId={browserTabId}
        live
      />,
    ),
  )
}

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

async function respond(status = 200) {
  const request = requests.at(-1)
  if (!request) throw new Error('No screenshot request')
  await act(async () =>
    request.resolve(
      new Response(new Blob(['jpeg'], { type: 'image/jpeg' }), { status }),
    ),
  )
}

async function decode() {
  const image = FakeImage.instances.at(-1)
  if (!image?.onload) throw new Error('No pending decode')
  await act(async () => image.onload?.())
}

async function setVisibility(value: DocumentVisibilityState) {
  await act(async () => {
    visibility = value
    document.dispatchEvent(new Event('visibilitychange'))
  })
}

const displayed = () =>
  container.querySelector('img')?.getAttribute('src') ?? null

describe('MiniScreencast bounded JPEG lifecycle', () => {
  it('renders a placeholder during SSR without fetching', () => {
    expect(
      renderToStaticMarkup(
        <MiniScreencast site="example.com" sessionId="one" />,
      ),
    ).toContain('example.com')
    expect(requests).toHaveLength(0)
  })

  it('does not start a screenshot request while the cockpit is hidden', async () => {
    visibility = 'hidden'
    await render()
    await advance(30_000)
    expect(requests).toHaveLength(0)
    expect(FakeImage.instances).toHaveLength(0)
    await setVisibility('visible')
    expect(requests).toHaveLength(1)
  })

  it('waits for fetch and decode before refreshing, with only current and pending frames', async () => {
    await render('session / one', 102)
    expect(requests[0].url).toBe(
      'http://127.0.0.1:9210/api/v1/sessions/session%20%2F%20one/preview?refresh=100&browserTabId=102',
    )
    await advance(3000)
    expect(requests).toHaveLength(1)
    await respond()
    await advance(3000)
    expect(requests).toHaveLength(1)
    expect(displayed()).toBeNull()
    await decode()
    expect(displayed()).toBe('blob:preview-1')
    expect(container.querySelector('.animate-pulse-dot')).not.toBeNull()
    for (let i = 0; i < 200; i++) {
      await advance(3000)
      await respond()
      expect(urls.size).toBe(2)
      await decode()
      expect(urls.size).toBe(1)
    }
    expect(requests).toHaveLength(201)
    expect(container.querySelectorAll('img')).toHaveLength(1)
    expect(
      FakeImage.instances.every(
        (image) =>
          image.onload === null && image.onerror === null && image.src === '',
      ),
    ).toBe(true)
  })

  it('retains the last good frame through HTTP and decode failures', async () => {
    await render()
    await respond()
    await decode()
    await advance(3000)
    await respond(503)
    expect(displayed()).toBe('blob:preview-1')
    await advance(3000)
    await respond()
    await act(async () => FakeImage.instances.at(-1)?.onerror?.())
    expect(displayed()).toBe('blob:preview-1')
    expect(urls.size).toBe(1)
    await advance(3000)
    await respond()
    await decode()
    expect(displayed()).toBe('blob:preview-3')
  })

  it('aborts in-flight requests and releases decoded frames while hidden', async () => {
    await render()
    await respond()
    await decode()
    await advance(3000)
    await setVisibility('hidden')
    expect(requests[1].signal.aborted).toBe(true)
    expect(displayed()).toBeNull()
    expect(urls.size).toBe(0)
    await advance(30_000)
    expect(requests).toHaveLength(2)
    await setVisibility('visible')
    expect(requests).toHaveLength(3)
  })

  it('discards old decode callbacks when switching tabs or sessions', async () => {
    await render('one', 101)
    await respond()
    const oldLoad = FakeImage.instances.at(-1)?.onload
    await render('one', 102)
    expect(requests[0].signal.aborted).toBe(true)
    expect(urls.size).toBe(0)
    await act(async () => oldLoad?.())
    expect(displayed()).toBeNull()
    expect(requests[1].url).toContain('browserTabId=102')
    await respond()
    await decode()
    await render('two', 102)
    expect(displayed()).toBeNull()
    expect(urls.size).toBe(0)
    expect(requests[2].url).toContain('/sessions/two/')
  })

  it('bounds hung network and decode work and retries after the deadline', async () => {
    await render()
    await advance(10_000)
    expect(requests[0].signal.aborted).toBe(true)
    await advance(3000)
    expect(requests).toHaveLength(2)
    await respond()
    expect(urls.size).toBe(1)
    await advance(10_000)
    expect(urls.size).toBe(0)
    await advance(3000)
    expect(requests).toHaveLength(3)
  })

  it('unmount cancels a pending decode and releases both image URLs', async () => {
    await render()
    await respond()
    await decode()
    await advance(3000)
    await respond()
    expect(urls.size).toBe(2)
    await act(async () => root.render(null))
    expect(urls.size).toBe(0)
    expect(timers.size).toBe(0)
    expect(requests.at(-1)?.signal.aborted).toBe(true)
  })
})
