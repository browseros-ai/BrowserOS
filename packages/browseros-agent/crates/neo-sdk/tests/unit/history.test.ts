/**
 * The messages `reload`, `goBack`, `goForward`, `waitForNavigation` and a predicate `waitForURL`
 * send, and what they make of Rust's replies, against a stub `__neo_runtime`. Each expectation is
 * Playwright's client behaviour at 26137face (`client/page.ts` `reload`, `goBack`, `goForward`;
 * `client/frame.ts` `waitForNavigation`, `waitForURL`, `verifyLoadState`;
 * `isomorphic/urlMatch.ts` `urlMatches`), or the wire contract of Neo's `neo:` navigation
 * commands (`src/neo-commands.ts`), where Playwright's client waits on events instead.
 */

import { beforeEach, describe, expect, test } from 'bun:test'
import { createRun } from '../../src/index'
import { eventless } from './bridge-stub'

type Sent = {
  guid: string
  method: string
  params: Record<string, unknown>
  timeout: number
}

/** A reply that never comes, as Rust's answer to a next navigation that has not happened. Like
 * Rust, the stub answers it with no navigation once its cursor's finish or release arrives. */
const HOLD = Symbol('hold')
let held: ((cursor: number) => void)[] = []
/** How late, in ms, the stub answers held calls after their cursor ended; and in what order the
 * held answers and the waits' settling came. */
let endDelay = 0
let order: string[] = []

let sent: Sent[] = []
let replies: unknown[] = []
/** The page's URL, as the live `url` getter answers it. */
let currentUrl = 'http://a.test/start'

/** Rust's `chain` answer for `response@T:D:0`: one request and its 200. */
const CHAIN = {
  requests: [
    {
      guid: 'request@T:D:0',
      initializer: {
        frame: { guid: 'frame@T' },
        url: 'http://a.test/next',
        resourceType: 'document',
        method: 'GET',
        headers: [],
        isNavigationRequest: true,
      },
    },
  ],
  responses: [
    {
      guid: 'response@T:D:0',
      initializer: {
        request: { guid: 'request@T:D:0' },
        url: 'http://a.test/next',
        status: 200,
        statusText: 'OK',
        headers: [],
        timing: {
          startTime: 1,
          domainLookupStart: -1,
          domainLookupEnd: -1,
          connectStart: -1,
          secureConnectionStart: -1,
          connectEnd: -1,
          requestStart: 0,
          responseStart: 0,
        },
        fromServiceWorker: false,
      },
    },
  ],
}

beforeEach(() => {
  sent = []
  replies = []
  held = []
  endDelay = 0
  order = []
  currentUrl = 'http://a.test/start'
  ;(globalThis as Record<string, unknown>).__neo_runtime = eventless({
    get: (request: string) => {
      const asked = JSON.parse(request)
      if (asked.field === 'parseURL') {
        // Bun's WHATWG parser stands in for Rust's.
        let url: URL
        try {
          url = new URL(asked.url)
        } catch {
          return JSON.stringify(null)
        }
        const { href, origin, protocol, username, password, host } = url
        const { hostname, port, pathname, search, hash } = url
        return JSON.stringify({
          href,
          origin,
          protocol,
          username,
          password,
          host,
          hostname,
          port,
          pathname,
          search,
          hash,
          searchParams: [...url.searchParams],
        })
      }
      if (asked.field === 'url') return JSON.stringify(currentUrl)
      if (asked.field === 'chain') return JSON.stringify(CHAIN)
      return JSON.stringify(null)
    },
    call: async (message: string) => {
      const { id, guid, method, params, metadata } = JSON.parse(message)
      sent.push({ guid, method, params, timeout: metadata.timeout })
      if (
        method === 'neo:finishNavigation' ||
        method === 'neo:releaseNavigation'
      ) {
        const ends = held.splice(0)
        const answer = () => {
          for (const end of ends) end(params.cursor)
        }
        if (endDelay) setTimeout(answer, endDelay)
        else answer()
      }
      const reply = replies.shift()
      // A call Rust keeps waiting, as the next navigation a predicate wait keeps asking for.
      if (reply === HOLD)
        return await new Promise<string>((resolve) => {
          held.push((cursor) => {
            order.push('answered')
            resolve(JSON.stringify({ id, result: { cursor } }))
          })
        })
      if (reply && typeof reply === 'object' && 'error' in reply)
        return JSON.stringify({ id, ...reply })
      return JSON.stringify({ id, result: reply ?? {} })
    },
  })
})

function run() {
  return createRun({
    page: 'page@T',
    mainFrame: 'frame@T',
    context: 'context@default',
    url: 'http://a.test/start',
    testIdAttribute: 'data-testid',
    lineOffset: 0,
  })
}

function wire() {
  return sent.map(({ guid, method, params, timeout }) => [
    guid,
    method,
    params,
    timeout,
  ])
}

describe('reload, goBack and goForward', () => {
  for (const method of ['reload', 'goBack', 'goForward'] as const) {
    test(`${method} sends its Page command with waitUntil load under the navigation timeout`, async () => {
      const { page } = run()
      page.setDefaultNavigationTimeout(7000)
      replies.push({ response: { guid: 'response@T:D:0' } }, {})
      const response = await page[method]()
      expect(response?.status()).toBe(200)
      expect(response?.url()).toBe('http://a.test/next')
      expect(
        await page[method]({ waitUntil: 'commit', timeout: 50 }),
      ).toBeNull()
      expect(wire()).toEqual([
        ['page@T', method, { waitUntil: 'load' }, 7000],
        ['page@T', method, { waitUntil: 'commit' }, 50],
      ])
    })
  }

  test('one guid is one Response, whichever call answered it', async () => {
    const { page } = run()
    replies.push(
      { response: { guid: 'response@T:D:0' } },
      { response: { guid: 'response@T:D:0' } },
    )
    const first = await page.reload()
    expect(await page.goBack()).toBe(first)
  })

  test('networkidle0 is sent as networkidle; an unknown waitUntil throws a plain Error first', async () => {
    const { page } = run()
    await page.goForward({ waitUntil: 'networkidle0' as never })
    expect(sent[0]?.params).toEqual({ waitUntil: 'networkidle' })
    const error = await page
      .reload({ waitUntil: 'later' as never })
      .catch((e) => e)
    expect(error.name).toBe('Error')
    expect(error.message).toBe(
      'waitUntil: expected one of (load|domcontentloaded|networkidle|commit)',
    )
    expect(sent).toHaveLength(1)
  })

  test("Rust's rejection keeps its kind and gains the API prefix", async () => {
    const { page } = run()
    replies.push({
      error: { error: { name: 'Error', message: 'net::ERR_EMPTY_RESPONSE' } },
    })
    const error = await page.reload().catch((e) => e)
    expect(error.name).toBe('Error')
    expect(error.message).toStartWith('page.reload: net::ERR_EMPTY_RESPONSE')
  })
})

describe('waitForNavigation', () => {
  test('with no url it sends the every-URL glob and resolves the Response or null', async () => {
    const { page } = run()
    replies.push({ response: { guid: 'response@T:D:0' } }, {})
    expect((await page.waitForNavigation())?.status()).toBe(200)
    expect(await page.mainFrame().waitForNavigation({ timeout: 10 })).toBeNull()
    expect(wire()).toEqual([
      [
        'frame@T',
        'neo:waitForNavigation',
        { url: { glob: '' }, waitUntil: 'load' },
        30000,
      ],
      [
        'frame@T',
        'neo:waitForNavigation',
        { url: { glob: '' }, waitUntil: 'load' },
        10,
      ],
    ])
  })

  test('a glob or RegExp crosses as one call, as waitForURL sends it', async () => {
    const { page } = run()
    await page.waitForNavigation({ url: '**/done', waitUntil: 'commit' })
    await page.waitForNavigation({ url: /done$/i })
    expect(sent.map(({ params }) => params)).toEqual([
      { url: { glob: '**/done' }, waitUntil: 'commit' },
      { url: { regexSource: 'done$', regexFlags: 'i' }, waitUntil: 'load' },
    ])
  })

  test('a predicate steps a cursor, keeps one next waiting and finishes the accepted navigation', async () => {
    const { page } = run()
    replies.push(
      { cursor: 4, index: 0, url: 'http://a.test/start#hash' },
      { cursor: 4, index: 1, url: 'http://a.test/next?q=a+b' },
      HOLD,
      { response: { guid: 'response@T:D:0' } },
    )
    const seen: unknown[] = []
    const response = await page.waitForNavigation({
      url: async (url) => {
        seen.push([url.pathname, url.searchParams.get('q'), String(url)])
        return url.pathname === '/next'
      },
      waitUntil: 'domcontentloaded',
    })
    expect(response?.status()).toBe(200)
    expect(seen).toEqual([
      ['/start', null, 'http://a.test/start#hash'],
      ['/next', 'a b', 'http://a.test/next?q=a+b'],
    ])
    expect(sent.map(({ method, params }) => [method, params])).toEqual([
      ['neo:nextNavigation', { response: true }],
      ['neo:nextNavigation', { cursor: 4, response: true }],
      ['neo:nextNavigation', { cursor: 4, response: true }],
      [
        'neo:finishNavigation',
        { cursor: 4, index: 1, waitUntil: 'domcontentloaded' },
      ],
    ])
  })

  test('a later navigation the predicate accepts wins while an earlier answer is pending', async () => {
    const { page } = run()
    replies.push(
      { cursor: 5, index: 0, url: 'http://a.test/first' },
      { cursor: 5, index: 1, url: 'http://a.test/goal' },
      HOLD,
      { response: { guid: 'response@T:D:0' } },
    )
    const response = await page.waitForNavigation({
      url: (url) =>
        url.pathname === '/first'
          ? new Promise<boolean>(() => {})
          : url.pathname === '/goal',
    })
    expect(response?.status()).toBe(200)
    expect(sent.at(-1)?.params).toEqual({
      cursor: 5,
      index: 1,
      waitUntil: 'load',
    })
  })

  test("the page's closure rejects a wait whose predicate is pending, with no timeout", async () => {
    const { page } = run()
    replies.push(
      { cursor: 6, index: 0, url: 'http://a.test/x' },
      {
        error: {
          error: {
            name: 'TargetClosedError',
            message: 'Target page, context or browser has been closed',
          },
        },
      },
    )
    const error = await page
      .waitForNavigation({
        url: () => new Promise<boolean>(() => {}),
        timeout: 0,
      })
      .catch((e) => e)
    expect(error.message).toContain('has been closed')
    expect(sent.map(({ method }) => method)).toEqual([
      'neo:nextNavigation',
      'neo:nextNavigation',
      'neo:releaseNavigation',
    ])
  })

  test("a predicate's URL is read-only and a URL that does not parse is never accepted", async () => {
    const { page } = run()
    replies.push(
      { cursor: 1, index: 0, url: 'not a url' },
      { cursor: 1, index: 1, url: 'http://a.test/x' },
      HOLD,
      {},
    )
    const urls: string[] = []
    await page.waitForNavigation({
      url: (url) => {
        urls.push(url.href)
        expect(() => {
          ;(url as { pathname: string }).pathname = '/y'
        }).toThrow(TypeError)
        return true
      },
    })
    expect(urls).toEqual(['http://a.test/x'])
  })

  test("a predicate's throw rejects with the API prefix and releases its cursor", async () => {
    const { page } = run()
    replies.push({ cursor: 2, index: 0, url: 'http://a.test/x' }, HOLD)
    const error = await page
      .waitForNavigation({
        url: () => {
          throw new RangeError('nope')
        },
      })
      .catch((e) => e)
    expect(error.name).toBe('RangeError')
    expect(error.message).toBe('page.waitForNavigation: nope')
    expect(sent.map(({ method, params }) => [method, params])).toEqual([
      ['neo:nextNavigation', { response: true }],
      ['neo:nextNavigation', { cursor: 2, response: true }],
      ['neo:releaseNavigation', { cursor: 2 }],
    ])
  })

  test('a wait that fails settles only after Rust answered the next its release ended', async () => {
    const { page } = run()
    endDelay = 30
    replies.push({ cursor: 7, index: 0, url: 'http://a.test/x' }, HOLD)
    const error = await page
      .waitForNavigation({
        url: () => {
          throw new RangeError('nope')
        },
      })
      .catch((e) => e)
    order.push('settled')
    expect(error.name).toBe('RangeError')
    expect(order).toEqual(['answered', 'settled'])
  })

  test('an accepted wait resolves only after Rust answered the next its finish ended', async () => {
    const { page } = run()
    endDelay = 30
    replies.push({ cursor: 8, index: 0, url: 'http://a.test/x' }, HOLD, {
      response: { guid: 'response@T:D:0' },
    })
    const response = await page.waitForNavigation({ url: () => true })
    order.push('settled')
    expect(response?.status()).toBe(200)
    expect(order).toEqual(['answered', 'settled'])
  })

  test('a step that times out names the whole timeout', async () => {
    const { page } = run()
    replies.push({
      error: {
        error: { name: 'TimeoutError', message: 'Timeout 12ms exceeded.' },
      },
    })
    const error = await page
      .waitForNavigation({ url: () => true, timeout: 5000 })
      .catch((e) => e)
    expect(error.name).toBe('TimeoutError')
    expect(error.message).toStartWith(
      'page.waitForNavigation: Timeout 5000ms exceeded.',
    )
  })

  test('a predicate still pending at the deadline rejects the wait then and releases it', async () => {
    const { page } = run()
    replies.push({ cursor: 3, index: 0, url: 'http://a.test/x' }, HOLD)
    const started = Date.now()
    const error = await page
      .waitForNavigation({
        url: () => new Promise<boolean>(() => {}),
        timeout: 50,
      })
      .catch((e) => e)
    expect(error.name).toBe('TimeoutError')
    expect(error.message).toStartWith(
      'page.waitForNavigation: Timeout 50ms exceeded.',
    )
    expect(Date.now() - started).toBeLessThan(5000)
    expect(sent.map(({ method }) => method)).toEqual([
      'neo:nextNavigation',
      'neo:nextNavigation',
      'neo:releaseNavigation',
    ])
  })

  test("an async predicate's rejection rejects with the API prefix", async () => {
    const { page } = run()
    replies.push({ cursor: 3, index: 0, url: 'http://a.test/x' }, HOLD)
    const error = await page
      .waitForNavigation({
        url: async () => Promise.reject(new TypeError('bad')),
      })
      .catch((e) => e)
    expect(error.name).toBe('TypeError')
    expect(error.message).toBe('page.waitForNavigation: bad')
    expect(sent.at(-1)?.method).toBe('neo:releaseNavigation')
  })

  test('an unknown waitUntil throws, prefixed, before anything is sent', async () => {
    const { page } = run()
    const error = await page
      .waitForNavigation({ waitUntil: 'never' as never })
      .catch((e) => e)
    expect(error.message).toBe(
      'page.waitForNavigation: waitUntil: expected one of (load|domcontentloaded|networkidle|commit)',
    )
    expect(sent).toEqual([])
  })
})

describe('waitForURL with a predicate', () => {
  test('a current URL the predicate accepts waits for the load state only', async () => {
    const { page } = run()
    await page.waitForURL((url) => url.pathname === '/start', {
      waitUntil: 'commit',
    })
    expect(sent.map(({ method, params }) => [method, params])).toEqual([
      ['neo:waitForLoadState', { state: 'commit' }],
    ])
  })

  test('otherwise it waits for the next accepted navigation, answering no Response', async () => {
    const { page } = run()
    replies.push({ cursor: 9, index: 0, url: 'http://a.test/done' }, HOLD, {})
    await page.waitForURL((url) => url.pathname === '/done')
    expect(sent.map(({ method, params }) => [method, params])).toEqual([
      ['neo:nextNavigation', { response: false }],
      ['neo:nextNavigation', { cursor: 9, response: false }],
      ['neo:finishNavigation', { cursor: 9, index: 0, waitUntil: 'load' }],
    ])
  })
})
