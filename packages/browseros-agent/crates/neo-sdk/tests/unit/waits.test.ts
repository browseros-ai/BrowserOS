/**
 * The messages `locator.waitFor`, `waitForURL` and `waitForLoadState` send, and what they make of
 * Rust's replies, against a stub `__neo_runtime`. Each expectation is Playwright's client
 * behaviour at 26137face (`client/locator.ts` `waitFor`, `client/frame.ts` `waitForURL`,
 * `waitForLoadState`, `verifyLoadState`, `isomorphic/urlMatch.ts` `serializeURLMatch`), or the
 * wire contract of Neo's own `neo:` commands (`src/neo-commands.ts`), where Playwright's client
 * waits on events instead.
 */

import { beforeEach, describe, expect, test } from 'bun:test'
import { createRun } from '../../src/index'

type Sent = {
  id: number
  guid: string
  method: string
  params: Record<string, unknown>
  metadata: { timeout: number; location: { file: string } }
}
type Reply = { result?: unknown; error?: { error: unknown }; log?: string[] }

let sent: Sent[] = []
let replies: Reply[] = []

beforeEach(() => {
  sent = []
  replies = []
  ;(globalThis as Record<string, unknown>).__neo_runtime = {
    get: () => JSON.stringify(null),
    call: async (message: string) => {
      const parsed: Sent = JSON.parse(message)
      sent.push(parsed)
      return JSON.stringify({ id: parsed.id, result: {}, ...replies.shift() })
    },
  }
})

function run() {
  return createRun({
    page: 'page@T',
    mainFrame: 'frame@T',
    context: 'context@1',
    url: 'about:blank',
    testIdAttribute: 'data-testid',
    lineOffset: 0,
  })
}

/** The message as JSON without `id` and `metadata.location`: key order is part of the wire. */
function wire(message: Sent | undefined): string {
  if (!message) return 'nothing sent'
  const { guid, method, params, metadata } = message
  return JSON.stringify({
    guid,
    method,
    params,
    timeout: metadata.timeout,
  })
}

function timedOut(timeout: number): Reply {
  return {
    error: {
      error: {
        name: 'TimeoutError',
        message: `Timeout ${timeout}ms exceeded.`,
      },
    },
    log: [],
  }
}

describe('locator.waitFor', () => {
  test('sends Frame.waitForSelector, strict, without a return value, and resolves undefined', async () => {
    const { page } = run()
    expect(await page.locator('#x').waitFor()).toBeUndefined()
    expect(wire(sent[0])).toBe(
      '{"guid":"frame@T","method":"waitForSelector","params":{"selector":"#x","strict":true,"omitReturnValue":true},"timeout":30000}',
    )
  })

  test('sends state in schema order, and no state when none is given (Rust defaults to visible)', async () => {
    const { page } = run()
    for (const state of ['attached', 'detached', 'visible', 'hidden'] as const)
      await page.getByRole('button').waitFor({ state })
    expect(sent.map(({ params }) => JSON.stringify(params))).toEqual(
      ['attached', 'detached', 'visible', 'hidden'].map(
        (state) =>
          `{"selector":"internal:role=button","strict":true,"state":"${state}","omitReturnValue":true}`,
      ),
    )
  })

  test('runs under the action timeout, never the navigation timeout', async () => {
    const { page, context } = run()
    const timeouts: number[] = []
    const wait = async (options?: { timeout?: number }) => {
      await page.locator('#x').waitFor(options)
      timeouts.push(sent[sent.length - 1]?.metadata.timeout ?? -1)
    }
    page.setDefaultNavigationTimeout(1)
    await wait()
    context.setDefaultTimeout(4)
    await wait()
    page.setDefaultTimeout(2)
    await wait()
    await wait({ timeout: 7 })
    await wait({ timeout: 0 })
    expect(timeouts).toEqual([30_000, 4, 2, 7, 0])
  })

  test("spreads the script's options last, as Playwright does, and drops unknown keys", async () => {
    const { page } = run()
    await page
      .locator('#x')
      .waitFor({ strict: false, timeout: 3, nope: 1 } as never)
    expect(wire(sent[0])).toBe(
      '{"guid":"frame@T","method":"waitForSelector","params":{"selector":"#x","strict":false,"omitReturnValue":true},"timeout":3}',
    )
  })

  test('a failed wait rebuilds the error with its apiName', async () => {
    const { page } = run()
    replies.push(timedOut(5))
    const error = await page
      .locator('#x')
      .waitFor({ timeout: 5 })
      .catch((e) => e)
    expect([error.name, error.message]).toEqual([
      'TimeoutError',
      'locator.waitFor: Timeout 5ms exceeded.',
    ])
  })
})

describe('waitForURL', () => {
  test('a string is a glob, sent as neo:waitForURL under the navigation timeout', async () => {
    const { page } = run()
    expect(await page.waitForURL('**/done.html')).toBeUndefined()
    expect(wire(sent[0])).toBe(
      '{"guid":"frame@T","method":"neo:waitForURL","params":{"url":{"glob":"**/done.html"},"waitUntil":"load"},"timeout":30000}',
    )
  })

  test('a RegExp is its source and flags, g and y included', async () => {
    const { page } = run()
    await page.waitForURL(/done\.html$/i, { waitUntil: 'commit' })
    await page.mainFrame().waitForURL(/a\/b/gy)
    expect(sent.map(({ params }) => JSON.stringify(params))).toEqual([
      '{"url":{"regexSource":"done\\\\.html$","regexFlags":"i"},"waitUntil":"commit"}',
      '{"url":{"regexSource":"a\\\\/b","regexFlags":"gy"},"waitUntil":"load"}',
    ])
  })

  test("'' and undefined match every URL, an empty String object none, as in Playwright's urlMatches", async () => {
    const { page } = run()
    await page.waitForURL('')
    await page.waitForURL(undefined as never)
    // A String object is a string to Playwright's `isString`, but only the primitive `''`
    // matches everything: `new String('')` goes on to `new RegExp('^$')`.
    await page.waitForURL(new String('**/x') as never)
    await page.waitForURL(new String('') as never)
    expect(sent.map(({ params }) => JSON.stringify(params.url))).toEqual([
      '{"glob":""}',
      '{"glob":""}',
      '{"glob":"**/x"}',
      '{"regexSource":"^$","regexFlags":""}',
    ])
  })

  test('every waitUntil is sent verified: networkidle0 is networkidle, which Rust refuses', async () => {
    const { page } = run()
    for (const waitUntil of [
      'load',
      'domcontentloaded',
      'commit',
      'networkidle',
      'networkidle0',
    ])
      await page.waitForURL('**', { waitUntil: waitUntil as never })
    expect(sent.map(({ params }) => params.waitUntil)).toEqual([
      'load',
      'domcontentloaded',
      'commit',
      'networkidle',
      'networkidle',
    ])
  })

  test('an unknown waitUntil throws before anything is sent', async () => {
    const { page } = run()
    const error = await page
      .waitForURL('**', { waitUntil: 'idle' as never })
      .catch((e) => e)
    expect([error.name, error.message]).toEqual([
      'Error',
      'page.waitForURL: waitUntil: expected one of (load|domcontentloaded|networkidle|commit)',
    ])
    expect(sent).toEqual([])
  })

  test('a predicate function is refused before anything is sent, naming the limit', async () => {
    const { page } = run()
    const error = await page
      .waitForURL(((url: URL) => url.pathname === '/done') as never)
      .catch((e) => e)
    expect(error.name).toBe('Error')
    expect(error.message).toStartWith('page.waitForURL: ')
    expect(error.message).toContain('predicate')
    const frameError = await page
      .mainFrame()
      .waitForURL((() => true) as never)
      .catch((e) => e)
    expect(frameError.message).toStartWith('frame.waitForURL: ')
    expect(sent).toEqual([])
  })

  test("any other url throws Playwright's own error before anything is sent", async () => {
    const { page } = run()
    for (const url of [42, null, {}, ['**'], true]) {
      const error = await page.waitForURL(url as never).catch((e) => e)
      expect([error.name, error.message]).toEqual([
        'Error',
        'url parameter should be string, RegExp, URLPattern or function',
      ])
    }
    expect(sent).toEqual([])
  })

  test('navigation defaults: page nav, page action, context nav, context action', async () => {
    const { page, context } = run()
    const timeouts: number[] = []
    const wait = async (options?: { timeout?: number }) => {
      await page.waitForURL('**', options)
      timeouts.push(sent[sent.length - 1]?.metadata.timeout ?? -1)
    }
    context.setDefaultTimeout(4)
    await wait()
    context.setDefaultNavigationTimeout(3)
    await wait()
    page.setDefaultTimeout(2)
    await wait()
    page.setDefaultNavigationTimeout(1)
    await wait()
    await wait({ timeout: 0 })
    expect(timeouts).toEqual([4, 3, 2, 1, 0])
  })

  test('frame.waitForURL sends the same message; errors carry each apiName', async () => {
    const { page } = run()
    replies.push(timedOut(9), timedOut(9))
    const pageError = await page
      .waitForURL('**/a', { timeout: 9 })
      .catch((e) => e)
    const frameError = await page
      .mainFrame()
      .waitForURL('**/a', { timeout: 9 })
      .catch((e) => e)
    expect(wire(sent[0])).toBe(wire(sent[1]))
    expect([pageError.message, frameError.message]).toEqual([
      'page.waitForURL: Timeout 9ms exceeded.',
      'frame.waitForURL: Timeout 9ms exceeded.',
    ])
    expect(frameError.name).toBe('TimeoutError')
  })
})

describe('waitForLoadState', () => {
  test('defaults to load, sent as neo:waitForLoadState under the navigation timeout', async () => {
    const { page } = run()
    expect(await page.waitForLoadState()).toBeUndefined()
    expect(wire(sent[0])).toBe(
      '{"guid":"frame@T","method":"neo:waitForLoadState","params":{"state":"load"},"timeout":30000}',
    )
  })

  test('sends each state verified: networkidle0 is networkidle, which Rust refuses', async () => {
    const { page } = run()
    for (const state of [
      'load',
      'domcontentloaded',
      'commit',
      'networkidle',
      'networkidle0',
    ])
      await page.waitForLoadState(state as never)
    await page.waitForLoadState(undefined, { timeout: 1 })
    expect(sent.map(({ params }) => params.state)).toEqual([
      'load',
      'domcontentloaded',
      'commit',
      'networkidle',
      'networkidle',
      'load',
    ])
  })

  test('an unknown state throws before anything is sent; only undefined defaults', async () => {
    const { page } = run()
    for (const state of ['idle', null]) {
      const error = await page.waitForLoadState(state as never).catch((e) => e)
      expect([error.name, error.message]).toEqual([
        'Error',
        'page.waitForLoadState: state: expected one of (load|domcontentloaded|networkidle|commit)',
      ])
    }
    expect(sent).toEqual([])
  })

  test('runs under the navigation timeout', async () => {
    const { page, context } = run()
    context.setDefaultTimeout(4)
    await page.waitForLoadState()
    page.setDefaultNavigationTimeout(1)
    await page.waitForLoadState('domcontentloaded')
    await page.mainFrame().waitForLoadState('commit', { timeout: 0 })
    expect(sent.map(({ metadata }) => metadata.timeout)).toEqual([4, 1, 0])
  })

  test('frame.waitForLoadState sends the same message; errors carry each apiName', async () => {
    const { page } = run()
    replies.push(timedOut(2), timedOut(2))
    const pageError = await page
      .waitForLoadState('load', { timeout: 2 })
      .catch((e) => e)
    const frameError = await page
      .mainFrame()
      .waitForLoadState('load', { timeout: 2 })
      .catch((e) => e)
    expect(wire(sent[0])).toBe(wire(sent[1]))
    expect([pageError.message, frameError.message]).toEqual([
      'page.waitForLoadState: Timeout 2ms exceeded.',
      'frame.waitForLoadState: Timeout 2ms exceeded.',
    ])
  })

  test('a closed page rejects as Playwright rebuilds a TargetClosedError', async () => {
    const { page } = run()
    replies.push({
      error: {
        error: {
          name: 'TargetClosedError',
          message: 'Target page, context or browser has been closed',
        },
      },
      log: [],
    })
    const error = await page.waitForLoadState().catch((e) => e)
    expect([error.name, error.message]).toEqual([
      'Error',
      'page.waitForLoadState: Target page, context or browser has been closed',
    ])
  })
})
