// Page and context events as scripts see them: listeners tell Rust what the run wants with a
// `since` stamp, one pump pulls each listener's events, `waitForEvent` stamps its call time
// before anything is sent and re-waits after a rejected event.
import { describe, expect, it } from 'bun:test'
import { type Call, json, loadPrelude, SEED } from './support/prelude-runtime'

const never = new Promise<never>(() => {})

/** Answers `events.next` from a queue of batches, then never again (the pump idles). */
function pumpAnswers(
  batches: unknown[][],
  others: (call: Call) => unknown = () => null,
) {
  const queue = [...batches]
  return (call: Call) => {
    if (call.name !== 'events.next') return others(call)
    const batch = queue.shift()
    return batch === undefined ? never : { events: batch, lost: 0 }
  }
}

const sleep = (ms: number) =>
  new Promise<void>((resolve) => setTimeout(resolve, ms))

/** The envelope Rust answers when an `events.wait` timed out after `timeoutMs`. */
const waitTimeout = (timeoutMs: number) => ({
  code: 'wait_timeout',
  owner: 'website',
  delivery: 'not_sent',
  retryable: true,
  timeout: true,
  message: `page.waitForEvent: Timeout ${timeoutMs}ms exceeded while waiting for event "popup"`,
})

async function eventually(check: () => boolean): Promise<void> {
  for (let attempt = 0; attempt < 100; attempt++) {
    if (check()) return
    await new Promise((resolve) => setTimeout(resolve, 5))
  }
  throw new Error('the condition never held')
}

const brand = (id: number) => ({ $neo: 'page', tab: 1000 + id })

/** The opener's `popup` event for a tab that opened. */
const popup = (seq: number, id: number, url: string, opener = 7) => ({
  seq,
  event: 'popup',
  page: brand(opener),
  popup: { page: brand(id), url },
})

/** The context's `page` event for a tab that opened. */
const newTab = (seq: number, id: number, url: string) => ({
  seq,
  event: 'page',
  page: brand(id),
  url,
  opener: null,
})

describe('waitForEvent', () => {
  it('tests that the wait is stamped when the script calls it, and its deadline counts from then', async () => {
    const runtime = loadPrelude({
      seed: SEED,
      answer: (call) =>
        call.name === 'events.wait'
          ? popup(4, 8, 'https://login.example/')
          : null,
    })
    const url = await runtime.run(`
      Date.now = () => 111
      const popup = page.waitForEvent('popup')
      Date.now = () => 999
      await page.getByText('Sign in').click()
      return (await popup).url()
    `)
    expect(url).toBe('https://login.example/')
    expect(runtime.calls.map((call) => call.name).sort()).toEqual([
      'events.wait',
      'locator.click',
    ])
    expect(runtime.sent('events.wait')[0]?.input).toEqual({
      call: { api: 'page.waitForEvent', timeoutMs: 111 + 5000 - 999 },
      page: { $neo: 'page', tab: 1007 },
      event: 'popup',
      since: 111,
      after: null,
    })
  })

  it('tests that a rejected record re-waits after its seq with the same stamp', async () => {
    const records = [
      popup(5, 8, 'https://ads.example/'),
      popup(9, 9, 'https://login.example/'),
    ]
    const runtime = loadPrelude({
      seed: SEED,
      answer: (call) => (call.name === 'events.wait' ? records.shift() : null),
    })
    const url = await runtime.run(`
      Date.now = () => 500
      const popup = await page.waitForEvent('popup', { predicate: (p) => p.url().includes('login'), timeout: 8000 })
      return popup.url()
    `)
    expect(url).toBe('https://login.example/')
    expect(runtime.sent('events.wait').map((call) => call.input)).toEqual([
      {
        call: { api: 'page.waitForEvent', timeoutMs: 8000 },
        page: { $neo: 'page', tab: 1007 },
        event: 'popup',
        since: 500,
        after: null,
      },
      {
        call: { api: 'page.waitForEvent', timeoutMs: 8000 },
        page: { $neo: 'page', tab: 1007 },
        event: 'popup',
        since: 500,
        after: 5,
      },
    ])
  })

  it("tests that a predicate's error rejects the wait with the method first", async () => {
    const runtime = loadPrelude({
      seed: SEED,
      answer: () => popup(3, 8, 'https://x.example/'),
    })
    const message = await runtime.run(
      "try { await page.waitForEvent('popup', () => { throw new Error('boom') }) } catch (e) { return e.message }",
    )
    expect(message).toBe('page.waitForEvent: boom')
  })

  it('tests that rejected events never restart the clock: a 30 ms wait ends at 30 ms', async () => {
    let waits = 0
    const runtime = loadPrelude({
      seed: SEED,
      answer: async (call) => {
        if (call.name !== 'events.wait') return null
        waits += 1
        if (waits <= 2) {
          await sleep(10)
          return popup(waits, 7 + waits, 'https://ads.example/')
        }
        // As Rust does: each op times out on its own clock, after the timeoutMs it was sent.
        const { timeoutMs } = call.input.call as { timeoutMs: number }
        await sleep(timeoutMs)
        return { err: waitTimeout(timeoutMs) }
      },
    })
    const started = performance.now()
    const thrown = json(
      await runtime.run(`
        try {
          await page.waitForEvent('popup', { predicate: () => false, timeout: 30 })
        } catch (e) { return [e.name, e.message] }
      `),
    )
    const took = performance.now() - started
    expect(thrown).toEqual([
      'TimeoutError',
      'page.waitForEvent: Timeout 30ms exceeded while waiting for event "popup"',
    ])
    expect(took).toBeLessThan(30 + 12)
    const sent = runtime
      .sent('events.wait')
      .map((call) => (call.input.call as { timeoutMs: number }).timeoutMs)
    expect(sent).toHaveLength(3)
    expect(sent[0]).toBeGreaterThan(sent[1] ?? 0)
    expect(sent[1]).toBeGreaterThan(sent[2] ?? 0)
  })

  it('tests that a predicate still pending at the deadline loses to it, as upstream', async () => {
    const runtime = loadPrelude({
      seed: SEED,
      answer: (call) =>
        call.name === 'events.wait'
          ? popup(1, 8, 'https://slow.example/')
          : null,
    })
    const started = performance.now()
    const thrown = json(
      await runtime.run(`
        const late = () => new Promise((resolve) => setTimeout(() => resolve(true), 200))
        try {
          await page.waitForEvent('popup', { predicate: late, timeout: 30 })
          return 'accepted'
        } catch (e) { return [e.name, e.message, e.code] }
      `),
    )
    expect(performance.now() - started).toBeLessThan(30 + 12)
    expect(thrown).toEqual([
      'TimeoutError',
      'page.waitForEvent: Timeout 30ms exceeded while waiting for event "popup"',
      'wait_timeout',
    ])
  })

  it('tests that a deadline passed while the predicate ran ends the wait with no re-wait', async () => {
    const runtime = loadPrelude({
      seed: SEED,
      answer: async (call) => {
        if (call.name !== 'events.wait') return null
        if (runtime.sent('events.wait').length === 1)
          return popup(1, 8, 'https://ads.example/')
        const { timeoutMs } = call.input.call as { timeoutMs: number }
        await sleep(timeoutMs)
        return { err: waitTimeout(timeoutMs) }
      },
    })
    const thrown = json(
      await runtime.run(`
        Date.now = () => 1_000
        const slow = () => { Date.now = () => 1_031; return false }
        try {
          await page.waitForEvent('popup', { predicate: slow, timeout: 30 })
        } catch (e) { return [e.name, e.message, e.code, e.owner, e.delivery, e.timeout] }
      `),
    )
    expect(thrown).toEqual([
      'TimeoutError',
      'page.waitForEvent: Timeout 30ms exceeded while waiting for event "popup"',
      'wait_timeout',
      'website',
      'not_sent',
      true,
    ])
    expect(runtime.sent('events.wait')).toHaveLength(1)
  })

  it('tests that the context waits for its page event with no page', async () => {
    const runtime = loadPrelude({
      seed: SEED,
      now: 1_000,
      answer: () => newTab(6, 30, 'https://new.example/'),
    })
    const listed = await runtime.run(
      "const tab = await context.waitForEvent('page'); return context.pages().includes(tab) && tab.url()",
    )
    expect(listed).toBe('https://new.example/')
    expect(runtime.sent('events.wait')[0]?.input).toMatchObject({
      call: { api: 'browserContext.waitForEvent', timeoutMs: 5000 },
      page: null,
      event: 'page',
    })
  })

  it('tests that events Neo never reports fail at once with a hint', async () => {
    const runtime = loadPrelude({ seed: SEED })
    const result = json(
      await runtime.run(`
        const errors = []
        try { await page.waitForEvent('request') } catch (e) { errors.push([e.code, e.message]) }
        try { page.on('framenavigated', () => {}) } catch (e) { errors.push([e.code, e.message]) }
        try { context.on('dialog', () => {}) } catch (e) { errors.push([e.code, e.message]) }
        return errors
      `),
    )
    expect(result).toEqual([
      [
        'method_unavailable',
        "page.waitForEvent('request'): Neo does not report network events; wait for what the page shows instead",
      ],
      [
        'method_unavailable',
        "page.on('framenavigated'): Neo does not report frame events; use page.waitForURL()",
      ],
      [
        'method_unavailable',
        'browserContext.on(\'dialog\'): Neo reports "dialog" on pages: page.on("dialog")',
      ],
    ])
    expect(runtime.calls.map((call) => [call.name, call.input.$neo])).toEqual([
      ["page.waitForEvent('request')", 'refused'],
      ["page.on('framenavigated')", 'refused'],
      ["browserContext.on('dialog')", 'refused'],
    ])
  })
})

describe('listeners and the pump', () => {
  it('tests that the first listener sends its interest with the time it was attached, and the last one removes it', async () => {
    const runtime = loadPrelude({ seed: SEED, answer: pumpAnswers([]) })
    await runtime.run(`
      Date.now = () => 2_000
      const log = () => {}
      page.on('console', log)
      page.on('console', () => {})
      Date.now = () => 3_000
      page.off('console', log)
      page.removeAllListeners('console')
      await new Promise((resolve) => setTimeout(resolve, 0))
    `)
    expect(runtime.sent('events.listen').map((call) => call.input)).toEqual([
      {
        call: { api: 'page.on', timeoutMs: 0 },
        page: { $neo: 'page', tab: 1007 },
        event: 'console',
        on: true,
        since: 2_000,
      },
      {
        call: { api: 'page.off', timeoutMs: 0 },
        page: { $neo: 'page', tab: 1007 },
        event: 'console',
        on: false,
        since: 3_000,
      },
    ])
    expect(runtime.sent('events.next')[0]?.input).toEqual({
      call: { api: 'events.next', timeoutMs: 0 },
    })
  })

  it('tests that a dialog reaches its listener, which answers it by id', async () => {
    const runtime = loadPrelude({
      seed: SEED,
      answer: pumpAnswers([
        [
          {
            seq: 3,
            event: 'dialog',
            page: brand(7),
            dialog: {
              id: 41,
              type: 'prompt',
              message: 'Your name?',
              defaultValue: 'Ann',
            },
          },
        ],
      ]),
    })
    const seen = json(
      await runtime.run(`
        return await new Promise((resolve) => page.on('dialog', async (dialog) => {
          await dialog.accept('Bo')
          resolve([dialog.type(), dialog.message(), dialog.defaultValue(), dialog.page() === page])
        }))
      `),
    )
    expect(seen).toEqual(['prompt', 'Your name?', 'Ann', true])
    expect(runtime.sent('dialog.accept')[0]?.input).toEqual({
      call: { api: 'dialog.accept', timeoutMs: 0 },
      page: { $neo: 'page', tab: 1007 },
      dialog: 41,
      promptText: 'Bo',
    })
    expect(runtime.sent('events.next')).toHaveLength(2)
  })

  it('tests that a dialog nobody listens for any more is dismissed, and beforeunload accepted, as upstream', async () => {
    const runtime = loadPrelude({
      seed: SEED,
      answer: pumpAnswers([
        [
          {
            seq: 1,
            event: 'dialog',
            page: brand(7),
            dialog: {
              id: 1,
              type: 'confirm',
              message: 'one',
              defaultValue: '',
            },
          },
          {
            seq: 2,
            event: 'dialog',
            page: brand(7),
            dialog: {
              id: 2,
              type: 'confirm',
              message: 'two',
              defaultValue: '',
            },
          },
          {
            seq: 3,
            event: 'dialog',
            page: brand(7),
            dialog: {
              id: 3,
              type: 'beforeunload',
              message: '',
              defaultValue: '',
            },
          },
        ],
      ]),
    })
    const first = await runtime.run(
      "return await new Promise((resolve) => page.once('dialog', (dialog) => resolve(dialog.message())))",
    )
    expect(first).toBe('one')
    await eventually(
      () =>
        runtime.sent('dialog.dismiss').length === 1 &&
        runtime.sent('dialog.accept').length === 1,
    )
    expect(runtime.sent('dialog.dismiss')[0]?.input.dialog).toBe(2)
    expect(runtime.sent('dialog.accept')[0]?.input.dialog).toBe(3)
    expect(runtime.sent('events.listen').map((call) => call.input.on)).toEqual([
      true,
      false,
    ])
  })

  it("tests that one opened tab reaches the opener's popup and the context's page as the same Page", async () => {
    const runtime = loadPrelude({
      seed: SEED,
      answer: pumpAnswers([
        [
          popup(8, 30, 'https://login.example/'),
          newTab(8, 30, 'https://login.example/'),
        ],
      ]),
    })
    const result = json(
      await runtime.run(`
        const popup = new Promise((resolve) => page.on('popup', resolve))
        const tab = new Promise((resolve) => context.on('page', resolve))
        const [fromPage, fromContext] = await Promise.all([popup, tab])
        return { same: fromPage === fromContext, url: fromPage.url(), listed: context.pages().includes(fromPage) }
      `),
    )
    expect(result).toEqual({
      same: true,
      url: 'https://login.example/',
      listed: true,
    })
    const listens = runtime
      .sent('events.listen')
      .map((call) => [call.input.page, call.input.event])
    expect(listens).toHaveLength(2)
    expect(listens).toContainEqual([{ $neo: 'page', tab: 1007 }, 'popup'])
    expect(listens).toContainEqual([null, 'page'])
  })

  it("tests that an event older than the pump's last one still reaches its listener, and a close closes once", async () => {
    const runtime = loadPrelude({
      seed: SEED,
      answer: pumpAnswers([
        [
          {
            seq: 9,
            event: 'console',
            page: brand(7),
            message: {
              type: 'log',
              text: 'after',
              location: {
                url: 'https://shop.example/a.js',
                lineNumber: 3,
                columnNumber: 9,
              },
              timestamp: 5,
            },
          },
        ],
        [{ seq: 4, event: 'close', page: brand(7) }],
        [{ seq: 4, event: 'close', page: brand(7) }],
      ]),
    })
    const result = json(
      await runtime.run(`
        let closes = 0
        page.on('console', () => {})
        const message = await new Promise((resolve) => page.on('console', resolve))
        const closed = new Promise((resolve) => page.on('close', () => { closes++; resolve() }))
        await closed
        await new Promise((resolve) => setTimeout(resolve, 5))
        return { closes, closed: page.isClosed(), text: message.text(), type: message.type(), location: message.location(), timestamp: message.timestamp() }
      `),
    )
    expect(result).toEqual({
      closes: 1,
      closed: true,
      text: 'after',
      type: 'log',
      location: {
        url: 'https://shop.example/a.js',
        line: 3,
        column: 9,
        lineNumber: 3,
        columnNumber: 9,
      },
      timestamp: 5,
    })
  })

  it("tests that pageerror carries the page's error and download its file", async () => {
    const runtime = loadPrelude({
      seed: SEED,
      answer: pumpAnswers(
        [
          [
            {
              seq: 1,
              event: 'pageerror',
              page: brand(7),
              error: {
                name: 'TypeError',
                message: 'x is undefined',
                stack: 'TypeError: x is undefined\n    at app.js:1',
              },
            },
            {
              seq: 2,
              event: 'download',
              page: brand(7),
              download: {
                id: 'guid-3',
                url: 'https://shop.example/r.pdf',
                suggestedFilename: 'r.pdf',
              },
            },
          ],
        ],
        (call) =>
          call.name === 'download.path'
            ? { path: '/runs/r1/r.pdf', failure: null }
            : null,
      ),
    })
    const result = json(
      await runtime.run(`
        const error = new Promise((resolve) => page.on('pageerror', resolve))
        const download = new Promise((resolve) => page.on('download', resolve))
        const [e, d] = await Promise.all([error, download])
        return { name: e.name, message: e.message, url: d.url(), file: d.suggestedFilename(), path: await d.path() }
      `),
    )
    expect(result).toEqual({
      name: 'TypeError',
      message: 'x is undefined',
      url: 'https://shop.example/r.pdf',
      file: 'r.pdf',
      path: '/runs/r1/r.pdf',
    })
    expect(runtime.sent('download.path')[0]?.input).toEqual({
      call: { api: 'download.path', timeoutMs: 0 },
      download: 'guid-3',
    })
  })

  /** A console message from the seed page. */
  const consoleEvent = (seq: number, text: string) => ({
    seq,
    event: 'console',
    page: brand(7),
    message: {
      type: 'log',
      text,
      location: { url: '', lineNumber: 0, columnNumber: 0 },
      timestamp: 1,
    },
  })

  it('tests that an op whose reply counts a pump reply resumes the script only after that reply ran its listeners', async () => {
    // The host answers the op first and the pump's reply after it, as the Node add-on can.
    let titled = () => {}
    const titleAnswered = new Promise<void>((resolve) => {
      titled = resolve
    })
    let pumps = 0
    const runtime = loadPrelude({
      seed: SEED,
      answer: async (call) => {
        if (call.name === 'page.title') {
          titled()
          return { ok: 'T', pumped: 1 }
        }
        if (call.name !== 'events.next') return null
        if (++pumps > 1) return never
        await titleAnswered
        await sleep(20)
        return { events: [consoleEvent(3, 'logged')], lost: 0 }
      },
    })
    const seen = json(
      await runtime.run(`
        const seen = []
        page.on('console', (message) => seen.push(message.text()))
        await page.title()
        seen.push('resumed')
        return seen
      `),
    )
    expect(seen).toEqual(['logged', 'resumed'])
  })

  it('tests that a pump reply that never comes holds an op only for its bound', async () => {
    const runtime = loadPrelude({
      seed: SEED,
      answer: (call) => {
        if (call.name === 'page.title') return { ok: 'T', pumped: 1 }
        return call.name === 'events.next' ? never : null
      },
    })
    const started = Date.now()
    const title = json(
      await runtime.run(`
        page.on('console', () => {})
        return await page.title()
      `),
    )
    expect(title).toBe('T')
    const held = Date.now() - started
    expect(held).toBeGreaterThanOrEqual(900)
    expect(held).toBeLessThan(5_000)
  })
})
