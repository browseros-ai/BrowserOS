/**
 * The realm half of the event pipe against a stub `__neo_runtime`: listener counts turn Rust
 * subscriptions on and off as Playwright's `ChannelOwner` does, a reply's events run before its
 * caller continues, the idle wake delivers while the script waits, unserved Playwright events
 * refuse a listener, and a listener error nobody handles ends the run. New pages, lifecycle and
 * frame events reach the page's and the context's listeners in Playwright's order, one object
 * per guid.
 */
import { beforeEach, expect, test } from 'bun:test'
import type { BrowserContext } from '../../src/browser-context'
import { EventEmitter } from '../../src/events'
import { createRun } from '../../src/index'
import type { Page } from '../../src/page'

type Subscription = { guid: string; event: string; enabled: boolean }

let subscriptions: Subscription[] = []
let queue: { seq: number; guid: string; method: string; params: unknown }[] = []
let seq = 0
let wake: (() => void) | undefined

function queueEvent(guid: string, method: string, params: unknown): number {
  queue.push({ seq: ++seq, guid, method, params })
  wake?.()
  wake = undefined
  return seq
}

/** A console event Rust would send for the run page. */
function consoleEvent(text: string) {
  return {
    type: 'log',
    text,
    args: [],
    location: { url: 'https://run.test/', lineNumber: 3, columnNumber: 7 },
    timestamp: 1,
    page: { guid: 'page@T' },
  }
}

let replyEvents: () => number = () => 0

beforeEach(() => {
  subscriptions = []
  queue = []
  seq = 0
  wake = undefined
  replyEvents = () => seq
  ;(globalThis as Record<string, unknown>).__neo_runtime = {
    get: () => JSON.stringify(null),
    callWithEvents: async (message: string) => ({
      reply: JSON.stringify({ id: JSON.parse(message).id }),
      events: replyEvents(),
    }),
    subscribe: (request: string) => {
      subscriptions.push(JSON.parse(request))
    },
    drain: (until: number) => {
      const taken = queue.filter((event) => until < 0 || event.seq <= until)
      queue = queue.filter((event) => !taken.includes(event))
      return JSON.stringify(taken)
    },
    nextEvents: () =>
      new Promise<void>((resolve) => {
        if (queue.length) resolve()
        else wake = resolve
      }),
  }
})

function run(): {
  page: Page
  context: BrowserContext
  uncaught: Promise<never>
} {
  return createRun({
    page: 'page@T',
    mainFrame: 'frame@T',
    context: 'context@default',
    url: 'https://run.test/',
    testIdAttribute: 'data-testid',
    lineOffset: 0,
  })
}

test('tests that the first listener subscribes and the last one unsubscribes', () => {
  const { page } = run()
  const first = () => {}
  const second = () => {}
  page.on('console', first)
  page.addListener('console', second)
  page.off('console', first)
  expect(subscriptions).toEqual([
    { guid: 'page@T', event: 'console', enabled: true },
  ])
  page.removeListener('console', second)
  expect(subscriptions.at(-1)).toEqual({
    guid: 'page@T',
    event: 'console',
    enabled: false,
  })
  page.once('dialog', () => {})
  page.prependListener('pageerror', () => {})
  page.removeAllListeners()
  expect(subscriptions.slice(2)).toEqual([
    { guid: 'page@T', event: 'dialog', enabled: true },
    { guid: 'page@T', event: 'pageerror', enabled: true },
    { guid: 'page@T', event: 'pageerror', enabled: false },
    { guid: 'page@T', event: 'dialog', enabled: false },
  ])
  // An event Neo has no producer for is an ordinary emitter event, as in Playwright.
  page.on('my-own-event', () => {})
  expect(subscriptions).toHaveLength(6)
  expect(page.emit('my-own-event')).toBe(true)
})

test('tests that an unserved Playwright event refuses a listener instead of never firing', async () => {
  const { page, context } = run()
  expect(() => page.on('request', () => {})).toThrow(
    'Neo does not serve the "request" event on Page yet',
  )
  expect(() => context.once('serviceworker', () => {})).toThrow(
    /BrowserContext/,
  )
  expect(page.listenerCount('request')).toBe(0)
  await expect(page.waitForEvent('download')).rejects.toThrow(
    'page.waitForEvent: Neo does not serve the "download" event on Page yet',
  )
  // The context never closes during a run: listening to it is allowed and silent.
  context.on('close', () => {})
  expect(subscriptions).toEqual([])
})

test('tests that a reply runs its events before the caller continues', async () => {
  const { page, context } = run()
  const order: string[] = []
  page.on('console', (message) => order.push(`page ${message.text()}`))
  context.on('console', (message) =>
    order.push(`context ${message.text()} ${message.page() === page}`),
  )
  replyEvents = () =>
    queueEvent('context@default', 'console', consoleEvent('a'))
  await page.bringToFront()
  order.push('continued')
  expect(order).toEqual(['page a', 'context a true', 'continued'])
})

test('tests that the idle wake delivers to waitForEvent and a timeout rejects', async () => {
  const { page } = run()
  const waited = page.waitForEvent(
    'console',
    (message) => message.text() === 'b',
  )
  queueEvent('context@default', 'console', consoleEvent('a'))
  queueEvent('context@default', 'console', consoleEvent('b'))
  const message = (await waited) as { text(): string; location(): unknown }
  expect(message.text()).toBe('b')
  expect(message.location()).toEqual({
    url: 'https://run.test/',
    line: 3,
    column: 7,
    lineNumber: 3,
    columnNumber: 7,
  })
  const error = await page
    .waitForEvent('console', { timeout: 5 })
    .catch((e: Error) => e)
  expect(error).toBeInstanceOf(Error)
  expect((error as Error).name).toBe('TimeoutError')
  expect((error as Error).message).toStartWith(
    'page.waitForEvent: Timeout 5ms exceeded while waiting for event "console"',
  )
  expect(page.listenerCount('console')).toBe(0)
  expect(page.listenerCount('close')).toBe(0)
})

test('tests that waitForEvent rejects when the page closes, unless it waits for close', async () => {
  const { page } = run()
  const waited = page.waitForEvent('console').catch((e: Error) => e)
  const closed = page.waitForEvent('close')
  queueEvent('page@T', 'close', {})
  expect(await closed).toBe(page)
  const error = (await waited) as Error
  expect(error.message).toStartWith(
    'page.waitForEvent: Target page, context or browser has been closed',
  )
  expect(() => page.waitForEvent('console', { signal: {} } as never)).toThrow()
})

test('tests that a dialog nobody listens to by then gets the default answer', async () => {
  const sent: { guid: string; method: string }[] = []
  const { page } = run()
  const native = (
    globalThis as unknown as Record<string, { callWithEvents: unknown }>
  ).__neo_runtime
  native.callWithEvents = async (message: string) => {
    const parsed = JSON.parse(message)
    sent.push({ guid: parsed.guid, method: parsed.method })
    return { reply: JSON.stringify({ id: parsed.id }), events: 0 }
  }
  const handler = () => {}
  page.on('dialog', handler)
  page.off('dialog', handler)
  queueEvent('context@default', 'dialog', {
    dialog: {
      guid: 'dialog@T:1',
      initializer: {
        page: { guid: 'page@T' },
        type: 'confirm',
        message: 'Sure?',
        defaultValue: '',
      },
    },
  })
  page.on('console', () => {})
  await new Promise((resolve) => setTimeout(resolve, 10))
  expect(sent).toEqual([{ guid: 'dialog@T:1', method: 'dismiss' }])
})

test('tests that a listener error nobody handles ends the run, sync or async', async () => {
  const sync = run()
  sync.page.on('console', () => {
    throw new TypeError('listener boom')
  })
  queueEvent('context@default', 'console', consoleEvent('a'))
  const syncError = await sync.uncaught.catch((e: Error) => e)
  expect((syncError as Error).name).toBe('TypeError')

  const handled = new EventEmitter()
  const seen: unknown[] = []
  handled.on('error', (error) => seen.push(error))
  handled.on('tick', async () => {
    throw new RangeError('caught by error listener')
  })
  handled.emit('tick')

  const asynchronous = run()
  asynchronous.page.on('console', async () => {
    throw new RangeError('async boom')
  })
  queueEvent('context@default', 'console', consoleEvent('b'))
  const asyncError = await asynchronous.uncaught.catch((e: Error) => e)
  expect((asyncError as Error).message).toBe('async boom')
  expect((seen[0] as Error).message).toBe('caught by error listener')
})

test('tests that removeAllListeners with wait waits for running async listeners', async () => {
  const { page } = run()
  let finished = false
  page.on('console', async () => {
    await new Promise((resolve) => setTimeout(resolve, 20))
    finished = true
  })
  replyEvents = () =>
    queueEvent('context@default', 'console', consoleEvent('a'))
  await page.bringToFront()
  expect(finished).toBe(false)
  await page.removeAllListeners('console', { behavior: 'wait' })
  expect(finished).toBe(true)
  expect(page.getMaxListeners()).toBe(0)
  expect(new EventEmitter().getMaxListeners()).toBe(10)
})

/** A frame record Rust would send (`frame_events.rs`). */
function frameRecord(guid: string, url: string, name: string) {
  return {
    guid,
    initializer: {
      url,
      name,
      parentFrame: { guid: 'frame@T' },
      loadStates: [],
    },
  }
}

/** A new page's record Rust would send (`tabs.rs` `Tabs::report_new`). */
function pageRecord(target: string, url: string, opener?: string) {
  return {
    guid: `page@${target}`,
    initializer: {
      mainFrame: { guid: `frame@${target}` },
      isClosed: false,
      ...(opener ? { opener: { guid: opener } } : {}),
    },
    mainFrame: {
      guid: `frame@${target}`,
      initializer: { url, name: '', loadStates: [] },
    },
  }
}

test('tests that a new page reaches the context, then its opener as a popup, as one object', async () => {
  const { page, context } = run()
  const order: string[] = []
  let fromContext: Page | undefined
  context.on('page', (created) => {
    fromContext = created
    order.push(`page ${created.url()}`)
  })
  page.on('popup', (popup) => order.push(`popup ${popup === fromContext}`))
  queueEvent('context@default', 'page', {
    page: pageRecord('P', 'https://popup.test/', 'page@T'),
  })
  // A page whose opener the run does not know is nobody's popup.
  queueEvent('context@default', 'page', {
    page: pageRecord('Q', 'about:blank'),
  })
  await page.bringToFront()
  expect(order).toEqual([
    'page https://popup.test/',
    'popup true',
    'page about:blank',
  ])
  expect(subscriptions).toEqual([
    { guid: 'context@default', event: 'page', enabled: true },
    { guid: 'page@T', event: 'popup', enabled: true },
  ])
})

test("tests that the main frame's load runs the page's listeners, then the context's", async () => {
  const { page, context } = run()
  const order: string[] = []
  page.on('domcontentloaded', (loaded) =>
    order.push(`domcontentloaded ${loaded === page}`),
  )
  page.on('load', (loaded) => order.push(`load ${loaded === page}`))
  context.on('pageload', (loaded) => order.push(`pageload ${loaded === page}`))
  queueEvent('page@T', 'domcontentloaded', {})
  queueEvent('page@T', 'load', {})
  await page.bringToFront()
  expect(order).toEqual(['domcontentloaded true', 'load true', 'pageload true'])
})

test('tests that frame events hand out one Frame per guid and a navigation updates it', async () => {
  const { page, context } = run()
  const seen: unknown[] = []
  let attached: unknown
  page.on('frameattached', (frame) => {
    attached = frame
    seen.push(['attached', frame.name()])
  })
  context.on('frameattached', (frame) =>
    seen.push(['context attached', frame === attached]),
  )
  page.on('framenavigated', (frame) =>
    seen.push(['navigated', frame === attached, frame.url(), frame.name()]),
  )
  context.on('framenavigated', (frame) =>
    seen.push(['context navigated', frame === page.mainFrame()]),
  )
  page.on('framedetached', (frame) =>
    seen.push(['detached', frame === attached]),
  )
  context.on('framedetached', (frame) =>
    seen.push(['context detached', frame === attached]),
  )
  queueEvent('page@T', 'frameAttached', {
    frame: frameRecord('frame@T/A', '', ''),
  })
  queueEvent('page@T', 'frameNavigated', {
    frame: frameRecord('frame@T/A', 'https://a.test/', 'x'),
    url: 'https://a.test/#f',
    name: 'x',
  })
  queueEvent('page@T', 'frameDetached', {
    frame: frameRecord('frame@T/A', 'https://a.test/#f', 'x'),
  })
  queueEvent('page@T', 'frameNavigated', {
    frame: {
      guid: 'frame@T',
      initializer: { url: 'https://run.test/next', name: '', loadStates: [] },
    },
    url: 'https://run.test/next',
    name: '',
  })
  await page.bringToFront()
  expect(seen).toEqual([
    ['attached', ''],
    ['context attached', true],
    ['navigated', true, 'https://a.test/#f', 'x'],
    ['context navigated', false],
    ['detached', true],
    ['context detached', true],
    ['navigated', false, 'https://run.test/next', ''],
    ['context navigated', true],
  ])
})

test("tests that a closing page runs its close listeners, then the context's pageclose", async () => {
  const { page, context } = run()
  const order: string[] = []
  page.on('close', (closed) => order.push(`close ${closed === page}`))
  context.on('pageclose', (closed) =>
    order.push(`pageclose ${closed === page}`),
  )
  queueEvent('page@T', 'close', {})
  await page.bringToFront()
  expect(order).toEqual(['close true', 'pageclose true'])
})
