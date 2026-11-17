// Playwright's `signal` option as scripts see it: an already-aborted signal sends nothing, one
// that fires mid-call aborts the op through the bootstrap's `$neo.abort`, and either way the
// script gets an `AbortError` whose `cause` is the signal's own reason. The script builds its
// signal by hand, as the prelude reads one by shape and never by class.
import { describe, expect, it } from 'bun:test'
import { type Call, json, loadPrelude, SEED } from './support/prelude-runtime'

/** A hand-made AbortSignal and its `abort`, defined in the script's realm. */
const SIGNAL = `
  const listeners = []
  const signal = {
    aborted: false,
    reason: undefined,
    addEventListener(type, listener) { listeners.push(listener) },
    removeEventListener(type, listener) {
      const at = listeners.indexOf(listener)
      if (at >= 0) listeners.splice(at, 1)
    },
  }
  const abort = (reason) => {
    signal.aborted = true
    signal.reason = reason
    for (const listener of [...listeners]) listener()
  }
  const tick = () => new Promise((resolve) => setTimeout(resolve, 10))
`

/** What Rust answers an op the script aborted: the leaf's message and call log, code `aborted`. */
const aborted = (op: string, reason: string) => ({
  err: {
    code: 'aborted',
    owner: 'script',
    delivery: 'not_sent',
    retryable: true,
    operationId: 'op.1',
    timeout: false,
    message: `${op}: ${reason}\nCall log:\n  - operation was aborted: ${reason}\n`,
  },
})

/** Answers `op` once the bootstrap aborts it, and every other op with null. */
const abortedWhenAsked = (op: string) => async (call: Call) => {
  if (call.name !== op || call.abort === undefined) return null
  return aborted(op, await call.abort.aborted)
}

describe('signal', () => {
  it('tests that an already-aborted signal sends nothing and throws AbortError with the reason as its cause', async () => {
    const runtime = loadPrelude({ seed: SEED })
    const outcome = json(
      await runtime.run(`${SIGNAL}
        const reason = new Error('Already aborted')
        abort(reason)
        const error = await page.locator('button').click({ signal }).catch((e) => e)
        return [error instanceof Error, error.name, error.message, error.cause === reason,
          error.code, error.delivery]
      `),
    )
    expect(outcome).toEqual([
      true,
      'AbortError',
      'locator.click: The operation was aborted',
      true,
      'aborted',
      'not_sent',
    ])
    expect(runtime.sent('locator.click')).toEqual([])
    expect(runtime.aborts).toEqual([])
  })

  it('tests that a signal firing mid-call aborts the op and the script gets an AbortError with the very reason as cause', async () => {
    const runtime = loadPrelude({
      seed: SEED,
      answer: abortedWhenAsked('locator.click'),
    })
    const outcome = json(
      await runtime.run(`${SIGNAL}
        const reason = new Error('foo bar')
        const pending = page.locator('button').click({ signal, timeout: 0 }).catch((e) => e)
        await tick()
        abort(reason)
        const error = await pending
        return [error.name, error.message, error.cause === reason, error.code, listeners.length]
      `),
    )
    expect(outcome).toEqual([
      'AbortError',
      'locator.click: foo bar\nCall log:\n  - operation was aborted: foo bar\n',
      true,
      'aborted',
      0,
    ])
    const [click] = runtime.sent('locator.click')
    expect(click?.abort?.key).toBeString()
    expect(runtime.aborts).toEqual([
      { key: click?.abort?.key ?? '', reason: 'foo bar' },
    ])
  })

  it('tests that a string reason reaches Rust as it is and stays the cause', async () => {
    const runtime = loadPrelude({
      seed: SEED,
      answer: abortedWhenAsked('locator.click'),
    })
    const outcome = json(
      await runtime.run(`${SIGNAL}
        const pending = page.click('button', { signal, timeout: 0 }).catch((e) => e)
        await tick()
        abort('aborted by user')
        const error = await pending
        return [error instanceof Error, error.name, error.message.split('\\n')[0], error.cause]
      `),
    )
    expect(outcome).toEqual([
      true,
      'AbortError',
      'page.click: aborted by user',
      'aborted by user',
    ])
    expect(runtime.aborts.map((sent) => sent.reason)).toEqual([
      'aborted by user',
    ])
  })

  it('tests that a signal aborted right after the call still finds it in flight, as upstream does (CL-091)', async () => {
    const runtime = loadPrelude({
      seed: SEED,
      answer: abortedWhenAsked('locator.click'),
    })
    const outcome = json(
      await runtime.run(`${SIGNAL}
        const pending = page.locator('button').click({ signal, timeout: 0 })
        abort('aborted by user')
        const error = await pending.catch((e) => e)
        return [error.name, error.message.split('\\n')[0], error.cause]
      `),
    )
    expect(outcome).toEqual([
      'AbortError',
      'locator.click: aborted by user',
      'aborted by user',
    ])
    expect(runtime.sent('locator.click')).toHaveLength(1)
    expect(runtime.aborts.map((sent) => sent.reason)).toEqual([
      'aborted by user',
    ])
  })

  it('tests that dragTo reads its signal at the call, before it brands the target', async () => {
    const runtime = loadPrelude({
      seed: SEED,
      answer: abortedWhenAsked('locator.dragTo'),
    })
    const outcome = json(
      await runtime.run(`${SIGNAL}
        const pending = page.locator('#a').dragTo(page.locator('#b'), { signal })
        abort('stop')
        const error = await pending.catch((e) => e)
        return [error.name, error.message.split('\\n')[0]]
      `),
    )
    expect(outcome).toEqual(['AbortError', 'locator.dragTo: stop'])
    expect(runtime.sent('locator.dragTo')).toHaveLength(1)
  })

  it('tests that waitForLoadState and waitForURL abort as upstream Waiter does, stopping the op in flight', async () => {
    const runtime = loadPrelude({
      seed: SEED,
      answer: async (call: Call) =>
        call.abort === undefined
          ? null
          : aborted(call.name, await call.abort.aborted),
    })
    const outcome = json(
      await runtime.run(`${SIGNAL}
        const reason = new Error('stop')
        const load = page.waitForLoadState('load', { signal }).catch((e) => e)
        const url = page.waitForURL('**/never', { signal }).catch((e) => e)
        await tick()
        abort(reason)
        return [await load, await url].map((error) =>
          [error.name, error.message, error.cause === reason])
      `),
    )
    expect(outcome).toEqual([
      ['AbortError', 'page.waitForLoadState: The operation was aborted', true],
      ['AbortError', 'page.waitForURL: The operation was aborted', true],
    ])
    expect(runtime.aborts.map((sent) => sent.reason)).toEqual(['stop', 'stop'])
  })

  it('tests that a screenshot with an aborted signal sends nothing', async () => {
    const runtime = loadPrelude({ seed: SEED })
    const outcome = json(
      await runtime.run(`${SIGNAL}
        abort('no picture')
        const error = await page.screenshot({ signal }).catch((e) => e)
        return [error.name, error.message, error.cause]
      `),
    )
    expect(outcome).toEqual([
      'AbortError',
      'page.screenshot: The operation was aborted',
      'no picture',
    ])
    expect(runtime.sent('page.screenshot')).toEqual([])
  })

  it('tests that a signal firing after the op answered sends no abort', async () => {
    const runtime = loadPrelude({ seed: SEED })
    const outcome = json(
      await runtime.run(`${SIGNAL}
        await page.locator('button').click({ signal })
        abort('too late')
        return listeners.length
      `),
    )
    expect(outcome).toBe(0)
    expect(runtime.aborts).toEqual([])
  })

  it('tests that a call without a signal is not made abortable', async () => {
    const runtime = loadPrelude({ seed: SEED })
    await runtime.run(`await page.locator('button').click()`)
    expect(runtime.sent('locator.click')[0]?.abort).toBeUndefined()
  })

  it('tests that navigation and waits forward their signal', async () => {
    const runtime = loadPrelude({ seed: SEED })
    await runtime.run(`${SIGNAL}
      await page.goto('https://example.com/', { signal })
      await page.waitForLoadState('load', { signal })
      await page.locator('button').fill('x', { signal })
    `)
    for (const op of ['page.goto', 'page.waitForLoadState', 'locator.fill'])
      expect(runtime.sent(op)[0]?.abort?.key).toBeString()
  })

  it('tests that waitForEvent rejects with upstream Waiter AbortError and aborts the events.wait in flight', async () => {
    const runtime = loadPrelude({
      seed: SEED,
      answer: abortedWhenAsked('events.wait'),
    })
    const outcome = json(
      await runtime.run(`${SIGNAL}
        const reason = new Error('stop')
        const pending = page.waitForEvent('popup', { signal }).catch((e) => e)
        await tick()
        abort(reason)
        const error = await pending
        return [error.name, error.message, error.cause === reason]
      `),
    )
    expect(outcome).toEqual([
      'AbortError',
      'page.waitForEvent: The operation was aborted',
      true,
    ])
    expect(runtime.aborts.map((sent) => sent.reason)).toEqual(['stop'])
  })
})
