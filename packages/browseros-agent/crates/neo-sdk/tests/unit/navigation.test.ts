/**
 * The messages `goto`, `title`, `textContent` and `ariaSnapshot` send, and what they make of
 * Rust's replies, against a stub `__neo_runtime`. Each expectation is Playwright's client
 * behaviour at 26137face (`client/frame.ts`, `client/page.ts`, `client/locator.ts`,
 * `client/timeoutSettings.ts`).
 */

import { beforeEach, describe, expect, test } from 'bun:test'
import { createRun } from '../../src/index'
import { eventless } from './bridge-stub'

type Sent = {
  id: number
  guid: string
  method: string
  params: Record<string, unknown>
  metadata: { timeout: number; location: { file: string } }
}

let sent: Sent[] = []
let results: unknown[] = []

beforeEach(() => {
  sent = []
  results = []
  ;(globalThis as Record<string, unknown>).__neo_runtime = eventless({
    get: () => JSON.stringify(null),
    call: async (message: string) => {
      const parsed: Sent = JSON.parse(message)
      sent.push(parsed)
      const result = results.shift()
      return JSON.stringify(
        result === undefined ? { id: parsed.id } : { id: parsed.id, result },
      )
    },
  })
})

function run() {
  return createRun({
    page: 'page@T',
    mainFrame: 'frame@T',
    context: 'context@1',
    request: 'request-context@default',
    playwright: 'playwright@default',
    url: 'about:blank',
    testIdAttribute: 'data-testid',
    lineOffset: 0,
  })
}

describe('goto', () => {
  test('sends url and waitUntil load under the navigation timeout, and resolves null', async () => {
    const { page } = run()
    results.push({})
    expect(await page.goto('http://localhost/a')).toBeNull()
    expect(
      sent.map(({ guid, method, params, metadata }) => [
        guid,
        method,
        params,
        metadata.timeout,
      ]),
    ).toEqual([
      [
        'frame@T',
        'goto',
        { url: 'http://localhost/a', waitUntil: 'load' },
        30_000,
      ],
    ])
  })

  test('keeps only schema params, in schema order, and maps networkidle0', async () => {
    const { page } = run()
    results.push({})
    await page.goto('http://x/', {
      referer: 'http://r/',
      timeout: 5,
      waitUntil: 'networkidle0' as never,
    })
    expect(JSON.stringify(sent[0]?.params)).toBe(
      '{"url":"http://x/","waitUntil":"networkidle","referer":"http://r/"}',
    )
    expect(sent[0]?.metadata.timeout).toBe(5)
  })

  test('an unknown waitUntil throws a plain Error and sends nothing', async () => {
    const { page } = run()
    const error = await page
      .goto('http://x/', { waitUntil: 'idle' as never })
      .catch((e) => e)
    expect([error.name, error.message]).toEqual([
      'Error',
      'waitUntil: expected one of (load|domcontentloaded|networkidle|commit)',
    ])
    expect(sent).toEqual([])
  })

  test('navigation defaults: page nav, page action, context nav, context action', async () => {
    const { page, context } = run()
    for (let i = 0; i < 5; i++) results.push({})
    const timeouts: number[] = []
    const go = async () => {
      await page.goto('http://x/')
      timeouts.push(sent[sent.length - 1]?.metadata.timeout ?? -1)
    }
    context.setDefaultTimeout(4)
    await go()
    context.setDefaultNavigationTimeout(3)
    await go()
    page.setDefaultTimeout(2)
    await go()
    page.setDefaultNavigationTimeout(1)
    await go()
    await page.goto('http://x/', { timeout: 0 })
    timeouts.push(sent[sent.length - 1]?.metadata.timeout ?? -1)
    expect(timeouts).toEqual([4, 3, 2, 1, 0])
  })

  test('frame.goto sends the same message', async () => {
    const { page } = run()
    results.push({})
    expect(
      await page.mainFrame().goto('about:blank', { waitUntil: 'commit' }),
    ).toBeNull()
    expect(sent[0]?.params).toEqual({ url: 'about:blank', waitUntil: 'commit' })
  })
})

describe('title', () => {
  test('sends no params and no timeout, whatever the defaults', async () => {
    const { page } = run()
    page.setDefaultTimeout(5)
    results.push({ value: 'Hello' }, { value: '' })
    expect(await page.title()).toBe('Hello')
    expect(await page.mainFrame().title()).toBe('')
    expect(
      sent.map(({ method, params, metadata }) => [
        method,
        params,
        metadata.timeout,
      ]),
    ).toEqual([
      ['title', {}, 0],
      ['title', {}, 0],
    ])
  })
})

describe('textContent', () => {
  test('locator.textContent is strict; page.textContent is not', async () => {
    const { page } = run()
    results.push({ value: 'a' }, { value: 'b' })
    expect(await page.locator('#x').textContent({ timeout: 7 })).toBe('a')
    expect(await page.textContent('#y')).toBe('b')
    expect(
      sent.map(({ params, metadata }) => [params, metadata.timeout]),
    ).toEqual([
      [{ selector: '#x', strict: true }, 7],
      [{ selector: '#y' }, 30_000],
    ])
  })

  test('an absent value reads as null, a present empty string stays', async () => {
    const { page } = run()
    results.push({}, { value: '' })
    expect(await page.locator('#x').textContent()).toBeNull()
    expect(await page.locator('#x').textContent()).toBe('')
  })

  test('uses the action timeout, not the navigation timeout', async () => {
    const { page } = run()
    page.setDefaultNavigationTimeout(1)
    results.push({ value: 'a' })
    await page.getByRole('button').textContent()
    expect(sent[0]?.metadata.timeout).toBe(30_000)
    expect(sent[0]?.params.selector).toBe('internal:role=button')
  })
})

describe('ariaSnapshot', () => {
  test('tests that a locator sends its scoped selector and returns the YAML', async () => {
    const { page } = run()
    results.push({ snapshot: '- button "Buy" [ref=e2]' })
    const section = page.getByRole('main').filter({ hasText: 'Products' })
    expect(
      await section.ariaSnapshot({
        mode: 'ai',
        depth: 2,
        boxes: true,
        timeout: 9,
      }),
    ).toBe('- button "Buy" [ref=e2]')
    expect(sent[0]).toMatchObject({
      guid: 'frame@T',
      method: 'ariaSnapshot',
      params: {
        selector: section._selector,
        mode: 'ai',
        depth: 2,
        boxes: true,
      },
      metadata: { timeout: 9 },
    })
  })

  test('sends mode, depth and boxes only, never a selector, and returns the YAML', async () => {
    const { page } = run()
    results.push({ snapshot: '- button "Go" [ref=e2]' }, { snapshot: '- main' })
    expect(await page.ariaSnapshot({ mode: 'ai', timeout: 9 })).toBe(
      '- button "Go" [ref=e2]',
    )
    expect(await page.ariaSnapshot({ depth: 2, boxes: true })).toBe('- main')
    expect(
      sent.map(({ method, params, metadata }) => [
        method,
        params,
        metadata.timeout,
      ]),
    ).toEqual([
      ['ariaSnapshot', { mode: 'ai' }, 9],
      ['ariaSnapshot', { depth: 2, boxes: true }, 30_000],
    ])
  })
})

describe('errors', () => {
  test('a failed read rebuilds the error with its apiName', async () => {
    const { page } = run()
    ;(globalThis as Record<string, unknown>).__neo_runtime = eventless({
      get: () => 'null',
      call: async (message: string) =>
        JSON.stringify({
          id: JSON.parse(message).id,
          error: {
            error: { name: 'TimeoutError', message: 'Timeout 5ms exceeded.' },
          },
          // Rust folds the log before it replies (`progress::compress_call_log`).
          log: ["  - waiting for locator('#x')"],
        }),
    })
    const error = await page
      .locator('#x')
      .textContent({ timeout: 5 })
      .catch((e) => e)
    expect([error.name, error.message]).toEqual([
      'TimeoutError',
      "locator.textContent: Timeout 5ms exceeded.\nCall log:\n  - waiting for locator('#x')\n",
    ])
  })
})
