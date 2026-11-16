// Page and context events as scripts see them: listeners tell Rust what the run wants with a
// `since` stamp, one pump pulls each listener's events, `waitForEvent` stamps its call time
// before anything is sent and re-waits after a rejected event.
import { describe, expect, it } from 'bun:test'
import type {
  DialogInfo,
  EventRecord,
  PageBrand,
} from '../src/prelude/generated'
import { type Call, json, loadPrelude, SEED } from './support/prelude-runtime'

const never = new Promise<never>(() => {})

/** Answers `events.next` from a queue of batches, then never again (the pump idles). */
function pumpAnswers(
  batches: EventRecord[][],
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

/**
 * When Rust recorded a fixture's event: after any listener's stamp, as every record the pump
 * delivers was (a context listener drops a record from before it attached).
 */
const RECORDED_AT = Number.MAX_SAFE_INTEGER

const brand = (id: number): PageBrand => ({ $neo: 'page', tab: 1000 + id })

/** The opener's `popup` event for a tab that opened. */
const popup = (
  seq: number,
  id: number,
  url: string,
  opener = 7,
): EventRecord => ({
  seq,
  at: RECORDED_AT,
  event: 'popup',
  page: brand(opener),
  popup: { page: brand(id), url },
})

/** A console message from the opener page. */
const consoleRecord = (seq: number): EventRecord => ({
  seq,
  at: RECORDED_AT,
  event: 'console',
  page: brand(7),
  message: {
    type: 'log',
    text: 'logged',
    args: [{ value: 'logged', preview: 'logged' }],
    location: { url: '', lineNumber: 0, columnNumber: 0 },
    timestamp: 1,
  },
})

/** The context's `page` event for a tab that opened. */
const newTab = (seq: number, id: number, url: string): EventRecord => ({
  seq,
  at: RECORDED_AT,
  event: 'page',
  page: brand(id),
  url,
  opener: null,
})

describe('waitForEvent', () => {
  it('tests that the wait is stamped when the script calls it, and its deadline counts from then', async () => {
    const runtime = loadPrelude({
      seed: SEED,
      // The run's page reads its whole tree at the start; the popup's own read gets none.
      answer: (call) =>
        call.name === 'events.wait'
          ? popup(4, 8, 'https://login.example/')
          : call.name === 'page.frames' &&
              Reflect.get(Object(call.input.page), 'tab') === 1007
            ? { frames: [{ id: 'main', parentId: null, name: '', url: '' }] }
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
    expect(
      runtime.calls
        .map((call) => call.name)
        .filter((name) => name !== 'page.frames')
        .sort(),
    ).toEqual(['events.wait', 'locator.click'])
    expect(runtime.sent('events.wait')[0]?.input).toEqual({
      call: { api: 'page.waitForEvent', timeoutMs: 111 + 30000 - 999 },
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

  it('tests that a later wait starts after the event the script last received, in the same millisecond', async () => {
    const records = [
      popup(5, 8, 'https://first.example/'),
      popup(9, 9, 'https://second.example/'),
    ]
    const runtime = loadPrelude({
      seed: SEED,
      answer: (call) => (call.name === 'events.wait' ? records.shift() : null),
    })
    const urls = await runtime.run(`
      Date.now = () => 500
      const first = await page.waitForEvent('popup')
      const second = await page.waitForEvent('popup')
      return [first.url(), second.url()]
    `)
    expect(urls).toEqual(['https://first.example/', 'https://second.example/'])
    expect(
      runtime
        .sent('events.wait')
        .map((call) => (call.input as { after: unknown }).after),
    ).toEqual([null, 5])
  })

  it('tests that a wait made by a listener the pump runs before the last wait resumed starts after that wait', async () => {
    let waited = () => {}
    const waitAnswered = new Promise<void>((resolve) => {
      waited = resolve
    })
    let waits = 0
    let pumps = 0
    const runtime = loadPrelude({
      seed: SEED,
      answer: async (call) => {
        if (call.name === 'events.wait') {
          waits += 1
          if (waits > 1) return never
          waited()
          return popup(10, 8, 'https://first.example/')
        }
        if (call.name !== 'events.next') return null
        if (++pumps > 1) return never
        await waitAnswered
        return { events: [consoleRecord(9)], lost: 0 }
      },
    })
    await runtime.run(`
      page.on('console', () => { page.waitForEvent('popup').catch(() => {}) })
      await page.waitForEvent('popup')
      await new Promise((resolve) => setTimeout(resolve, 20))
    `)
    expect(
      runtime
        .sent('events.wait')
        .map((call) => (call.input as { after: unknown }).after),
    ).toEqual([null, 10])
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
      call: { api: 'browserContext.waitForEvent', timeoutMs: 30000 },
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
        try { context.on('console', () => {}) } catch (e) { errors.push([e.code, e.message]) }
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
        'browserContext.on(\'console\'): Neo reports "console" on pages: page.on("console")',
      ],
    ])
    expect(runtime.calls.map((call) => [call.name, call.input.$neo])).toEqual([
      ["page.waitForEvent('request')", 'refused'],
      ["browserContext.on('console')", 'refused'],
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
        enabled: true,
        since: 2_000,
      },
      {
        call: { api: 'page.off', timeoutMs: 0 },
        page: { $neo: 'page', tab: 1007 },
        event: 'console',
        enabled: false,
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
            at: RECORDED_AT,
            event: 'dialog',
            page: brand(7),
            dialog: {
              id: 41,
              type: 'prompt',
              message: 'Your name?',
              defaultValue: 'Ann',
            },
            context: false,
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

  it("tests that a dismissal sends only Dialog.dismiss's params and both answers resolve to nothing", async () => {
    const runtime = loadPrelude({
      seed: SEED,
      answer: pumpAnswers([
        [
          {
            seq: 3,
            at: RECORDED_AT,
            event: 'dialog',
            page: brand(7),
            dialog: {
              id: 42,
              type: 'confirm',
              message: 'Sure?',
              defaultValue: '',
            },
            context: false,
          },
        ],
      ]),
    })
    // Both answers are void on the wire (`null`) and resolve to `undefined` in the script; an
    // argument passed to dismiss() never reaches the wire, as upstream.
    const answered = await runtime.run(`
      return await new Promise((resolve) => page.on('dialog', async (dialog) => {
        resolve(String(await dialog.dismiss('ignored')))
      }))
    `)
    expect(answered).toBe('undefined')
    expect(runtime.sent('dialog.dismiss')[0]?.input).toEqual({
      call: { api: 'dialog.dismiss', timeoutMs: 0 },
      page: { $neo: 'page', tab: 1007 },
      dialog: 42,
    })
  })

  it("tests that a filechooser listener leases interception with Page.updateSubscription's params", async () => {
    const runtime = loadPrelude({ seed: SEED, answer: () => null })
    await runtime.run(`
      const chooser = () => {}
      page.on('filechooser', chooser)
      page.off('filechooser', chooser)
      // A write waits for interception's setup and release (chooser-setup.ts).
      await page.getByText('Upload').click()
    `)
    expect(
      runtime.sent('events.interceptFileChooser').map((call) => call.input),
    ).toEqual([
      {
        call: { api: 'page.filechooser', timeoutMs: 0 },
        page: { $neo: 'page', tab: 1007 },
        event: 'fileChooser',
        enabled: true,
        lease: 1,
      },
      {
        call: { api: 'page.filechooser', timeoutMs: 0 },
        page: { $neo: 'page', tab: 1007 },
        event: 'fileChooser',
        enabled: false,
        lease: 1,
      },
    ])
  })

  it('tests that a dialog nobody listens for any more is dismissed, and beforeunload accepted, as upstream', async () => {
    const runtime = loadPrelude({
      seed: SEED,
      answer: pumpAnswers([
        [
          {
            seq: 1,
            at: RECORDED_AT,
            event: 'dialog',
            page: brand(7),
            dialog: {
              id: 1,
              type: 'confirm',
              message: 'one',
              defaultValue: '',
            },
            context: false,
          },
          {
            seq: 2,
            at: RECORDED_AT,
            event: 'dialog',
            page: brand(7),
            dialog: {
              id: 2,
              type: 'confirm',
              message: 'two',
              defaultValue: '',
            },
            context: false,
          },
          {
            seq: 3,
            at: RECORDED_AT,
            event: 'dialog',
            page: brand(7),
            dialog: {
              id: 3,
              type: 'beforeunload',
              message: '',
              defaultValue: '',
            },
            context: false,
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
    expect(
      runtime.sent('events.listen').map((call) => call.input.enabled),
    ).toEqual([true, false])
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

  it("tests that util.inspect prints a console message's text and a locator's description, as upstream", async () => {
    const runtime = loadPrelude({
      seed: SEED,
      answer: pumpAnswers([
        [
          {
            seq: 2,
            at: RECORDED_AT,
            event: 'console',
            page: brand(7),
            message: {
              type: 'log',
              text: 'Hello world',
              args: [{ value: 'Hello world', preview: 'Hello world' }],
              location: { url: '', lineNumber: 0, columnNumber: 0 },
              timestamp: 1,
            },
          },
        ],
      ]),
    })
    const inspected = json(
      await runtime.run(`
        const custom = Symbol.for('nodejs.util.inspect.custom')
        const message = await new Promise((resolve) => page.on('console', resolve))
        return [message[custom](), page.locator('#name')[custom]()]
      `),
    )
    expect(inspected).toEqual(['Hello world', "locator('#name')"])
  })

  it('tests that console arguments answer jsonValue() with the value Rust read, and refuse what it could not', async () => {
    const runtime = loadPrelude({
      seed: SEED,
      answer: pumpAnswers([
        [
          {
            seq: 2,
            at: RECORDED_AT,
            event: 'console',
            page: brand(7),
            message: {
              type: 'log',
              text: 'hello 5 {foo: bar} {a: {…}}',
              args: [
                { value: 'hello', preview: 'hello' },
                { value: 5, preview: '5' },
                {
                  value: { o: [{ k: 'foo', v: 'bar' }], id: 1 },
                  preview: '{foo: bar}',
                },
                { preview: '{a: {…}}' },
              ],
              location: { url: '', lineNumber: 0, columnNumber: 0 },
              timestamp: 1,
            },
          },
        ],
      ]),
    })
    const read = json(
      await runtime.run(`
        const message = await new Promise((resolve) => page.on('console', resolve))
        const args = message.args()
        const values = await Promise.all(args.slice(0, 3).map((arg) => arg.jsonValue()))
        const refused = await args[3].jsonValue().then(() => null, (e) => [e.code, e.message])
        const evaluated = await args[0].evaluate((v) => v).then(() => null, (e) => e.code)
        return { values, refused, evaluated, printed: args.map(String) }
      `),
    )
    expect(read).toEqual({
      values: ['hello', 5, { foo: 'bar' }],
      refused: [
        'method_unavailable',
        expect.stringContaining('jsHandle.jsonValue: the console reported'),
      ],
      evaluated: 'method_unavailable',
      printed: ['hello', '5', '{foo: bar}', '{a: {…}}'],
    })
  })

  it("tests that an event older than the pump's last one still reaches its listener, and a close closes once", async () => {
    const runtime = loadPrelude({
      seed: SEED,
      answer: pumpAnswers([
        [
          {
            seq: 9,
            at: RECORDED_AT,
            event: 'console',
            page: brand(7),
            message: {
              type: 'log',
              text: 'after',
              args: [],
              location: {
                url: 'https://shop.example/a.js',
                lineNumber: 3,
                columnNumber: 9,
              },
              timestamp: 5,
            },
          },
        ],
        [{ seq: 4, at: RECORDED_AT, event: 'close', page: brand(7) }],
        [{ seq: 4, at: RECORDED_AT, event: 'close', page: brand(7) }],
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
              at: RECORDED_AT,
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
              at: RECORDED_AT,
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
        return {
          ok: { events: [consoleEvent(3, 'logged')], lost: 0 },
          pumped: 1,
        }
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

  it('tests that a failed pump reply Rust never counted does not release a later hold early', async () => {
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
        pumps += 1
        // The first read failed before Rust counted it; the restarted pump's reply is the first
        // counted one, and the host settles it after the op's.
        if (pumps === 1)
          return {
            err: {
              code: 'internal_error',
              owner: 'neo',
              delivery: 'not_sent',
              retryable: false,
              message: 'the read failed',
            },
          }
        if (pumps > 2) return never
        await titleAnswered
        await sleep(20)
        return {
          ok: { events: [consoleEvent(3, 'logged')], lost: 0 },
          pumped: 1,
        }
      },
    })
    const seen = json(
      await runtime.run(`
        const seen = []
        page.on('dialog', () => {})
        await new Promise((resolve) => setTimeout(resolve, 10))
        page.on('console', (message) => seen.push(message.text()))
        await page.title()
        seen.push('resumed')
        return seen
      `),
    )
    expect(pumps).toBeGreaterThanOrEqual(2)
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

// The wire delivers independent page/context subscriptions. Their payloads must still share
// the canonical Frame when a wait and the pump observe the same journal record.
describe('page lifecycle events', () => {
  it('keeps frame identity across listeners, waits, snapshots and removal', async () => {
    let next: ((value: unknown) => void) | undefined
    const frame = {
      id: 'child',
      parentId: 'main',
      name: 'kid',
      url: 'https://frame.test/',
    }
    const record = (seq: number, event: string, context = false) => ({
      seq,
      at: Date.now(),
      event,
      page: brand(7),
      frame,
      context,
    })
    const runtime = loadPrelude({
      seed: SEED,
      answer: (call) => {
        if (call.name === 'events.next')
          return new Promise((resolve) => {
            next = resolve
          })
        if (call.name === 'events.wait') return record(11, 'framedetached')
        return null
      },
    })
    const pending = runtime.run(`
      const received = []
      let attached
      page.once('frameattached', frame => { attached = frame; received.push(frame.page() === page && frame.parentFrame() === page.mainFrame()) })
      context.once('frameattached', frame => received.push(frame === attached))
      const removedListener = () => received.push('unexpected')
      page.on('frameattached', removedListener)
      page.off('frameattached', removedListener)
      await new Promise(resolve => page.once('frameattached', resolve))
      const detached = await page.waitForEvent('framedetached', { predicate: frame => frame.isDetached() })
      return { received, same: detached === attached, detached: detached.isDetached(), listed: page.frames().includes(detached) }
    `)
    await eventually(() => next !== undefined)
    next?.({
      events: [record(10, 'frameattached'), record(10, 'frameattached', true)],
      lost: 0,
    })
    expect(json(await pending)).toEqual({
      received: [true, true],
      same: true,
      detached: true,
      listed: false,
    })
  })

  it('returns Page for load events and retains one deadline across predicate rejection', async () => {
    let candidate = 0
    const runtime = loadPrelude({
      seed: SEED,
      answer: (call) => {
        if (call.name === 'events.wait')
          return {
            seq: ++candidate,
            at: Date.now(),
            event: call.input.event,
            page: brand(7),
            frame: {
              id: 'main',
              parentId: null,
              name: '',
              url: 'https://loaded.test/',
            },
          }
        return null
      },
    })
    expect(
      json(
        await runtime.run(`
      let seen = 0
      const loaded = await page.waitForEvent('load', { predicate: p => { if (p !== page || p.url() !== 'https://loaded.test/') throw new Error('wrong page state'); return ++seen === 2 }, timeout: 500 })
      const dom = await page.waitForEvent('domcontentloaded')
      const contextLoad = await context.waitForEvent('pageload')
      return { seen, same: loaded === page && dom === page && contextLoad === page }
    `),
      ),
    ).toEqual({ seen: 2, same: true })
    const calls = runtime.calls.filter((call) => call.name === 'events.wait')
    expect(calls[0]?.input.since).toBe(calls[1]?.input.since)
    expect(calls[1]?.input.after).toBe(1)
  })
})

it('does not let an older frame event resurrect a frame or replace a newer URL', async () => {
  let candidate = 0
  const runtime = loadPrelude({
    seed: SEED,
    answer: (call) => {
      if (call.name !== 'events.wait') return null
      const records = [
        {
          seq: 11,
          event: 'framedetached',
          frame: {
            id: 'child',
            parentId: 'main',
            name: '',
            url: 'https://child.test/',
          },
        },
        {
          seq: 10,
          event: 'frameattached',
          frame: { id: 'child', parentId: 'main', name: '', url: '' },
        },
        {
          seq: 15,
          event: 'framenavigated',
          frame: {
            id: 'main',
            parentId: null,
            name: '',
            url: 'https://new.test/',
          },
        },
        {
          seq: 13,
          event: 'framenavigated',
          frame: {
            id: 'main',
            parentId: null,
            name: '',
            url: 'https://old.test/',
          },
        },
      ]
      return {
        ...records[candidate++],
        page: brand(7),
        at: Date.now(),
        context: false,
      }
    },
  })
  expect(
    json(
      await runtime.run(`
    const detached = await page.waitForEvent('framedetached')
    const attached = await page.waitForEvent('frameattached')
    const gone = detached === attached && detached.isDetached() && !page.frames().includes(detached)
    await page.waitForEvent('framenavigated')
    await page.waitForEvent('framenavigated')
    return { gone, url: page.url() }
  `),
    ),
  ).toEqual({ gone: true, url: 'https://new.test/' })
})

it('checks each navigation predicate against its event when the pump has advanced', async () => {
  let push: ((value: unknown) => void) | undefined
  let candidate: ((value: unknown) => void) | undefined
  let pulls = 0
  const record = (seq: number, url: string) => ({
    seq,
    at: Date.now(),
    event: 'framenavigated',
    page: brand(7),
    context: false,
    frame: { id: 'main', parentId: null, name: '', url },
  })
  const runtime = loadPrelude({
    seed: SEED,
    answer: (call) => {
      if (call.name === 'events.next') {
        pulls++
        return new Promise((resolve) => {
          push = resolve
        })
      }
      if (call.name === 'events.wait')
        return new Promise((resolve) => {
          candidate = resolve
        })
      return null
    },
  })
  const pending = runtime.run(`
    const callbacks = []
    let matched
    page.on('framenavigated', frame => callbacks.push(frame.url()))
    const frame = await page.waitForEvent('framenavigated', { timeout: 1000, predicate: frame => { matched = frame.url(); return matched.endsWith('#target') } })
    return { callbacks, matched, same: frame === page.mainFrame(), latest: page.url() }
  `)
  await eventually(() => push !== undefined && candidate !== undefined)
  const first = record(21, 'https://a.test/#target')
  push?.({ events: [first, record(22, 'https://a.test/#later')], lost: 0 })
  await eventually(() => pulls === 2)
  candidate?.(first)
  expect(json(await pending)).toEqual({
    callbacks: ['https://a.test/#target', 'https://a.test/#later'],
    matched: 'https://a.test/#target',
    same: true,
    latest: 'https://a.test/#later',
  })
})

it('orders context on/off around async setup and rejects an old pump period', async () => {
  let finishSetup: (() => void) | undefined
  let push: ((value: unknown) => void) | undefined
  let pulls = 0
  const changes: unknown[] = []
  const runtime = loadPrelude({
    seed: SEED,
    answer: (call) => {
      if (call.name === 'events.listen') {
        changes.push(call.input.enabled)
        if (changes.length === 1)
          return new Promise<void>((resolve) => {
            finishSetup = resolve
          })
      }
      if (call.name === 'events.next') {
        pulls++
        return new Promise((resolve) => {
          push = resolve
        })
      }
      return null
    },
  })
  let done = false
  const pending = runtime
    .run(`
    const old = () => { throw new Error('removed listener fired') }
    context.on('pageload', old)
    context.off('pageload', old)
    return await new Promise(resolve => context.once('pageload', value => resolve(value === page && value.url())))
  `)
    .then((value) => {
      done = true
      return value
    })
  await eventually(() => finishSetup !== undefined && push !== undefined)
  expect(changes).toEqual([true])
  finishSetup?.()
  await eventually(() => changes.length >= 3)
  expect(changes.slice(0, 3)).toEqual([true, false, true])
  const load = (seq: number, at: number, url: string) => ({
    seq,
    at,
    event: 'pageload',
    page: brand(7),
    frame: { id: 'main', parentId: null, name: '', url },
  })
  push?.({
    events: [load(30, Date.now() - 1000, 'https://old.test/')],
    lost: 0,
  })
  await eventually(() => pulls === 2)
  expect(done).toBe(false)
  push?.({ events: [load(31, Date.now(), 'https://new.test/')], lost: 0 })
  expect(await pending).toBe('https://new.test/')
})

it('reuses the original Dialog for page and context closure and delivers each once', async () => {
  const recordedAt = Date.now()
  const dialog: DialogInfo = {
    id: 91,
    type: 'confirm',
    message: 'Close?',
    defaultValue: '',
  }
  const runtime = loadPrelude({
    seed: SEED,
    now: recordedAt,
    answer: pumpAnswers([
      [
        {
          seq: 1,
          at: recordedAt,
          event: 'dialog',
          page: brand(7),
          dialog,
          context: false,
        },
        {
          seq: 2,
          at: recordedAt,
          event: 'dialogclosed',
          page: brand(7),
          dialog,
          context: true,
        },
        {
          seq: 2,
          at: recordedAt,
          event: 'dialogclosed',
          page: brand(7),
          dialog,
          context: false,
        },
      ],
    ]),
  })
  expect(
    json(
      await runtime.run(`
    let opened
    const order = []
    page.once('dialog', dialog => opened = dialog)
    context.once('dialogclosed', dialog => order.push(dialog === opened ? 'context' : 'wrong'))
    const closed = await new Promise(resolve => page.once('dialogclosed', resolve))
    order.push(closed === opened ? 'page' : 'wrong')
    return { order, message: closed.message(), page: closed.page() === page, listeners: page.listenerCount('dialogclosed') }
  `),
    ),
  ).toEqual({
    order: ['context', 'page'],
    message: 'Close?',
    page: true,
    listeners: 0,
  })
})

it('delivers an automatically closed dialog to waits without answering it again', async () => {
  const runtime = loadPrelude({
    seed: SEED,
    answer: (call) =>
      call.name === 'events.wait'
        ? {
            seq: 3,
            at: Date.now(),
            event: 'dialogclosed',
            page: brand(7),
            context: call.input.page === null,
            dialog: {
              id: 92,
              type: 'alert',
              message: 'automatic',
              defaultValue: '',
            },
          }
        : null,
  })
  expect(
    json(
      await runtime.run(`
    const closed = await page.waitForEvent('dialogclosed', dialog => dialog.message() === 'automatic')
    const mirrored = await context.waitForEvent('dialogclosed')
    return closed === mirrored && closed.type() === 'alert' && closed.page() === page
  `),
    ),
  ).toBe(true)
  expect(
    runtime.calls.some(
      (call) => call.name === 'dialog.accept' || call.name === 'dialog.dismiss',
    ),
  ).toBe(false)
})

it('delivers worker console before snapshot closure and preserves the worker entry URL', async () => {
  const worker = { id: 'worker-1', url: 'https://a.example/entry.js' }
  let release = () => {}
  const ready = new Promise<void>((resolve) => {
    release = resolve
  })
  let pulled = false
  const runtime = loadPrelude({
    seed: {
      ...SEED,
      pages: [{ tab: 1007, url: 'https://a.example/', workers: [worker] }],
    },
    now: 1000,
    answer: (call) => {
      if (call.name === 'events.next') {
        if (pulled) return never
        pulled = true
        return ready.then(() => ({
          ok: {
            events: [
              {
                seq: 1,
                at: 1001,
                event: 'console',
                page: brand(7),
                message: {
                  type: 'log',
                  text: 'last message',
                  worker,
                  location: {
                    url: 'https://a.example/imported.js',
                    lineNumber: 0,
                    columnNumber: 0,
                  },
                  timestamp: 1001,
                },
              },
              {
                seq: 2,
                at: 1001,
                event: 'workerstate',
                page: brand(7),
                worker,
                closed: true,
              },
            ],
            lost: 0,
          },
          pages: [{ tab: 1007, workers: [] }],
          observed: { seq: 2, events: 2 },
        }))
      }
      if (call.name === 'page.evaluate') {
        release()
        return { value: { v: 'undefined' } }
      }
      return null
    },
  })
  const result = json(
    await runtime.run(`
    const worker = page.workers()[0]
    const order = []
    worker.on('console', message => order.push(message.worker() === worker && message.worker().url()))
    worker.on('close', () => order.push('close'))
    const message = worker.waitForEvent('console')
    const filtered = worker.waitForEvent('console', message => message.text() === 'last message')
    await page.evaluate(() => undefined)
    return { text: (await message).text(), filtered: (await filtered).text(), order, count: page.workers().length }
  `),
  )
  expect(result).toEqual({
    text: 'last message',
    filtered: 'last message',
    order: [worker.url, 'close'],
    count: 0,
  })
})

it('releases internal worker bookkeeping once when a page closes', async () => {
  let sequence = 0
  const runtime = loadPrelude({
    seed: { ...SEED, pages: [{ tab: 1007, closed: false, workers: [] }] },
    answer: (call) => {
      if (call.name === 'events.next') return never
      if (call.name === 'page.close' || call.name === 'page.title')
        return {
          ok: call.name === 'page.title' ? { title: 'closed' } : {},
          observed: { seq: ++sequence, events: sequence },
          pages: [{ tab: 1007, closed: true, workers: [] }],
        }
      return null
    },
  })
  await runtime.run('await page.close(); await page.title()')
  await eventually(() =>
    runtime
      .sent('events.listen')
      .some(
        (call) =>
          call.input.event === 'workerstate' && call.input.enabled === false,
      ),
  )
  expect(
    [...runtime.startCalls, ...runtime.calls]
      .filter(
        (call) =>
          call.name === 'events.listen' && call.input.event === 'workerstate',
      )
      .map((call) => call.input.enabled),
  ).toEqual([true, false])
  const cleanup = runtime
    .sent('events.listen')
    .find((call) => call.input.enabled === false)
  expect(cleanup?.input.page).toBeNull()
  expect(cleanup?.input.retiredTab).toBe(1007)
})

describe('removeAllListeners with a behavior', () => {
  /** A console listener that is still running, as page-listeners.spec.ts sets one up. */
  const running = (fails: string | null) => `
    let release
    const released = new Promise(resolve => { release = resolve })
    let value = 0
    await new Promise(resolve => page.on('console', async () => {
      resolve()
      await released
      value = 42
      ${fails === null ? '' : `throw new Error(${JSON.stringify(fails)})`}
    }))
  `

  it("tests that 'wait' settles once the running listener has, and rejects with its error", async () => {
    const runtime = loadPrelude({
      seed: SEED,
      answer: pumpAnswers([[consoleRecord(3)]]),
    })
    const result = json(
      await runtime.run(`
      ${running('Error in handler')}
      const removed = page.removeAllListeners('console', { behavior: 'wait' })
      const left = page.listenerCount('console')
      release()
      const error = await removed.then(() => null, (e) => e.message)
      return { left, error, value }
    `),
    )
    expect(result).toEqual({ left: 0, error: 'Error in handler', value: 42 })
    expect(
      runtime.sent('events.listen').map((call) => call.input.enabled),
    ).toEqual([true, false])
  })

  it("tests that 'wait' resolves after a listener that succeeds, and no options keep Node's chaining", async () => {
    const runtime = loadPrelude({
      seed: SEED,
      answer: pumpAnswers([[consoleRecord(3)]]),
    })
    const result = json(
      await runtime.run(`
      ${running(null)}
      const removed = page.removeAllListeners('console', { behavior: 'wait' })
      release()
      await removed
      const settled = value
      const plain = page.removeAllListeners('console') === page
      const others = await page.removeAllListeners(undefined, { behavior: 'default' })
      return { settled, plain, others: String(others) }
    `),
    )
    expect(result).toEqual({ settled: 42, plain: true, others: 'undefined' })
  })

  // Bun fails a test on an unhandled rejection, from the vm realm too (measured: the same
  // script with `behavior: 'default'` fails with the listener's error), so passing is the proof.
  it("tests that 'ignoreErrors' silences a removed listener's later failure", async () => {
    const runtime = loadPrelude({
      seed: SEED,
      answer: pumpAnswers([[consoleRecord(3)]]),
    })
    const result = await runtime.run(`
      ${running('Error in console handler')}
      const removed = page.removeAllListeners('console', { behavior: 'ignoreErrors' })
      release()
      return String(await removed)
    `)
    await sleep(20)
    expect(result).toBe('undefined')
  })
})

describe('dialogs on the context, dialog waits, crashes', () => {
  const dialogRecord = (
    seq: number,
    tab: number,
    context: boolean,
    id = 51,
  ): EventRecord => ({
    seq,
    at: RECORDED_AT,
    event: 'dialog',
    page: brand(tab),
    dialog: { id, type: 'confirm', message: 'hello', defaultValue: '' },
    context,
  })

  it("tests that the context's dialog wait and listener get a popup's dialog on the popup, and the pump leaves it to them", async () => {
    const runtime = loadPrelude({
      seed: SEED,
      answer: pumpAnswers([[dialogRecord(5, 8, true)]], (call) => {
        if (call.name !== 'events.wait') return null
        return call.input.event === 'popup'
          ? popup(4, 8, 'about:blank')
          : dialogRecord(5, 8, true)
      }),
    })
    const result = json(
      await runtime.run(`
      const heard = new Promise(resolve => context.once('dialog', resolve))
      const [popup, dialog] = await Promise.all([
        page.waitForEvent('popup'),
        context.waitForEvent('dialog'),
      ])
      return {
        message: dialog.message(),
        onPopup: dialog.page() === popup,
        same: (await heard) === dialog,
      }
    `),
    )
    expect(result).toEqual({ message: 'hello', onPopup: true, same: true })
    // The context's wait needs no page brand, so it may go out first.
    expect(
      runtime
        .sent('events.wait')
        .map((call) => [call.input.page, call.input.event])
        .sort((a, b) => String(a[1]).localeCompare(String(b[1]))),
    ).toEqual([
      [null, 'dialog'],
      [{ $neo: 'page', tab: 1007 }, 'popup'],
    ])
    expect(
      runtime
        .sent('events.listen')
        .map((call) => [call.input.page, call.input.event, call.input.enabled]),
    ).toEqual([
      [null, 'dialog', true],
      [null, 'dialog', false],
    ])
    await sleep(20)
    expect(
      runtime.calls.some(
        (call) =>
          call.name === 'dialog.accept' || call.name === 'dialog.dismiss',
      ),
    ).toBe(false)
  })

  it("tests that a context dialog queued for a listener that left is still answered, as upstream's client does", async () => {
    let push: ((value: unknown) => void) | undefined
    const runtime = loadPrelude({
      seed: SEED,
      answer: (call) =>
        call.name === 'events.next'
          ? new Promise((resolve) => {
              push = resolve
            })
          : null,
    })
    await runtime.run(`
      const listener = () => { throw new Error('a removed listener heard it') }
      context.on('dialog', listener)
      context.off('dialog', listener)
    `)
    await eventually(() => push !== undefined)
    push?.({ events: [dialogRecord(5, 7, true, 71)], lost: 0 })
    await eventually(() => runtime.sent('dialog.dismiss').length === 1)
    expect(runtime.sent('dialog.dismiss')[0]?.input.dialog).toBe(71)
  })

  it('tests that a dialog a wait takes is never answered by the pump, though its listener left', async () => {
    let answerWait: ((record: EventRecord) => void) | undefined
    const runtime = loadPrelude({
      seed: SEED,
      answer: pumpAnswers([[dialogRecord(3, 7, false, 61)]], (call) =>
        call.name === 'events.wait'
          ? new Promise((resolve) => {
              answerWait = resolve
            })
          : null,
      ),
    })
    const pending = runtime.run(`
      const waited = page.waitForEvent('dialog')
      const listener = () => {}
      page.on('dialog', listener)
      page.off('dialog', listener)
      const dialog = await waited
      await dialog.accept()
      return dialog.message()
    `)
    await eventually(
      () =>
        answerWait !== undefined && runtime.sent('events.next').length === 1,
    )
    // The pump delivered the dialog to a page with no listener while the wait was in flight;
    // without the wait it dismisses it (the test of a dialog nobody listens for any more).
    await sleep(20)
    expect(runtime.sent('dialog.dismiss')).toHaveLength(0)
    answerWait?.(dialogRecord(3, 7, false, 61))
    expect(await pending).toBe('hello')
    expect(
      runtime.sent('dialog.accept').map((call) => call.input.dialog),
    ).toEqual([61])
    expect(runtime.sent('dialog.dismiss')).toHaveLength(0)
  })

  it("tests that page.on('crash') hears its page crash and waitForEvent('crash') resolves with the page", async () => {
    const crash: EventRecord = {
      seq: 3,
      at: RECORDED_AT,
      event: 'crash',
      page: brand(7),
    }
    const runtime = loadPrelude({
      seed: SEED,
      answer: pumpAnswers([[crash]], (call) =>
        call.name === 'events.wait' ? crash : null,
      ),
    })
    const result = json(
      await runtime.run(`
      const heard = new Promise(resolve => page.once('crash', resolve))
      const waited = await page.waitForEvent('crash')
      return [(await heard) === page, waited === page, page.isClosed()]
    `),
    )
    expect(result).toEqual([true, true, false])
    expect(runtime.sent('events.listen')[0]?.input).toMatchObject({
      page: { $neo: 'page', tab: 1007 },
      event: 'crash',
      enabled: true,
    })
  })
})
