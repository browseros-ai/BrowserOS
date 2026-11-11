// `exposeBinding` and `exposeFunction` in the prelude: what registering sends, upstream's
// duplicate checks, and how a page's call that the pump delivers is answered: the callback run
// with its source and the page's arguments, and its result or error handed back in the calling
// context through `bindingCall.resolve`.
import { describe, expect, it } from 'bun:test'
import { type Call, loadPrelude, SEED } from './support/prelude-runtime'

const never = new Promise(() => {})

/** A page's call of `name` with Playwright in-page arguments, as Rust delivers it. */
function call(name: string, serializedArgs: unknown, seq = 1) {
  return {
    seq: 9,
    at: 1,
    event: 'binding',
    page: { $neo: 'page', tab: 1007 },
    frame: null,
    executionContext: 4,
    payload: JSON.stringify({ name, seq, serializedArgs }),
  }
}

/** Registrations answer an id; the pump hands out `batch` once, then idles. */
function runtime(batch: unknown[] = []) {
  let pulled = false
  return loadPrelude({
    seed: SEED,
    answer: (sent: Call) => {
      if (sent.name === 'events.next') {
        if (pulled) return never
        pulled = true
        return { events: batch, lost: 0 }
      }
      if (
        sent.name.endsWith('.exposeBinding') ||
        sent.name.endsWith('.addInitScript')
      )
        return { id: 3 }
      return {}
    },
  })
}

const settle = () => new Promise((resolve) => setTimeout(resolve, 20))

describe('registering', () => {
  it('tests that exposeFunction sends the page binding and a duplicate fails before any op', async () => {
    const run = runtime()
    const thrown = await run.run(`
      await page.exposeFunction('add', (a, b) => a + b)
      try { await page.exposeBinding('add', () => 0); return null } catch (error) { return error.message }
    `)
    expect(thrown).toBe(
      'page.exposeBinding: Function "add" has been already registered',
    )
    expect(run.sent('page.exposeBinding').map((sent) => sent.input)).toEqual([
      {
        call: { api: 'page.exposeFunction', timeoutMs: 0 },
        page: { $neo: 'page', tab: 1007 },
        name: 'add',
      },
      {
        $neo: 'refused',
        code: 'invalid_input',
        reason:
          'page.exposeBinding: Function "add" has been already registered',
      },
    ])
  })

  it("tests that the context refuses a page's name and the page refuses the context's", async () => {
    const run = runtime()
    const messages = await run.run(`
      const failed = async (call) => { try { await call(); return null } catch (error) { return error.message } }
      await page.exposeFunction('a', () => 1)
      await context.exposeFunction('b', () => 1)
      return [
        await failed(() => context.exposeFunction('a', () => 1)),
        await failed(() => page.exposeFunction('b', () => 1)),
      ]
    `)
    expect(messages).toEqual([
      'browserContext.exposeFunction: Function "a" has been already registered in one of the pages',
      'page.exposeFunction: Function "b" has been already registered in the browser context',
    ])
  })

  it('tests that exposeFunctions exposes each function off window before the script that calls it', async () => {
    const run = runtime()
    await run.run(`
      Math.random = () => 0.5
      await page.addInitScript(({ cb }) => cb(1), { cb: () => 2 }, { exposeFunctions: true })
    `)
    const name = `__pw_fn_${'80'.repeat(16)}`
    expect(run.sent('page.exposeBinding').map((sent) => sent.input)).toEqual([
      {
        call: { api: 'page.addInitScript', timeoutMs: 0 },
        page: { $neo: 'page', tab: 1007 },
        name,
        noGlobal: true,
      },
    ])
    expect(
      run.sent('page.addInitScript').map((sent) => sent.input.script),
    ).toEqual([
      {
        source: `(({ cb }) => cb(1))(globalThis['__playwright__binding__controller__'].parseInitScriptArg({"o":[{"k":"cb","v":{"fn":"${name}"}}],"id":1}))`,
      },
    ])
  })
})

describe('answering a call', () => {
  it('tests that the callback gets its source and arguments, and its result goes back where the call came from', async () => {
    const run = runtime([call('add', [2, 3])])
    const seen = await run.run(`
      let seen = null
      await page.exposeBinding('add', (source, a, b) => {
        seen = [source.page === page, source.frame === page.mainFrame(), source.context === context, a, b]
        return a + b
      })
      await new Promise((resolve) => setTimeout(resolve, 20))
      return seen
    `)
    await settle()
    expect(seen).toEqual([true, true, true, 2, 3])
    expect(run.sent('bindingCall.resolve').map((sent) => sent.input)).toEqual([
      {
        call: { api: 'bindingCall.resolve', timeoutMs: 0 },
        page: { $neo: 'page', tab: 1007 },
        context: 4,
        arg: JSON.stringify({
          o: [
            { k: 'name', v: 'add' },
            { k: 'seq', v: 1 },
            { k: 'result', v: 5 },
          ],
          id: 1,
        }),
      },
    ])
  })

  it("tests that what the callback throws goes back as the call's error", async () => {
    const run = runtime([call('boom', [])])
    await run.run(`
      await page.exposeFunction('boom', () => { throw new TypeError('no') })
      await new Promise((resolve) => setTimeout(resolve, 20))
    `)
    await settle()
    const [answer] = run.sent('bindingCall.resolve')
    const arg = JSON.parse(String(answer?.input.arg))
    expect(arg.o[2].k).toBe('error')
    expect(arg.o[2].v.e).toMatchObject({ n: 'TypeError', m: 'no' })
  })

  it('tests that a name the run never exposed is left unanswered, for the run that did', async () => {
    const run = runtime([call('theirs', [])])
    await run.run(`
      await page.exposeFunction('mine', () => 1)
      await new Promise((resolve) => setTimeout(resolve, 20))
    `)
    await settle()
    expect(run.sent('bindingCall.resolve')).toEqual([])
  })
})

it('tests that a call with a known frame runs its callback as the pump dispatches it, before the next record', async () => {
  // Upstream's client calls the function as it dispatches the call, so whatever the pump
  // dispatches after it (here a console listener) runs after the callback.
  let release = () => {}
  const released = new Promise<void>((resolve) => {
    release = resolve
  })
  let pulled = false
  const run = loadPrelude({
    seed: SEED,
    answer: (sent: Call) => {
      if (sent.name === 'events.next') {
        if (pulled) return never
        pulled = true
        return released.then(() => ({
          events: [
            call('f', []),
            {
              seq: 10,
              at: Date.now() + 60_000,
              event: 'console',
              page: { $neo: 'page', tab: 1007 },
              message: {
                type: 'log',
                text: 'x',
                location: { url: '', lineNumber: 0, columnNumber: 0 },
                timestamp: 0,
              },
            },
          ],
          lost: 0,
        }))
      }
      if (sent.name === 'events.listen') release()
      if (sent.name === 'page.exposeBinding') return { id: 3 }
      return {}
    },
  })
  const order = await run.run(`
    const order = []
    await page.exposeFunction('f', () => { order.push('callback') })
    page.on('console', () => order.push('console'))
    await new Promise((resolve) => setTimeout(resolve, 20))
    return order
  `)
  expect(order).toEqual(['callback', 'console'])
})

it('tests that disposing twice does not remove the same name exposed again since', async () => {
  const run = runtime([call('f', [])])
  await run.run(`
    const cb = () => 1
    const first = await page.exposeFunction('f', cb)
    await first.dispose()
    await page.exposeFunction('f', cb)
    await first.dispose()
    await new Promise((resolve) => setTimeout(resolve, 20))
  `)
  await settle()
  expect(run.sent('bindingCall.resolve')).toHaveLength(1)
})

it('tests that an answer from a registration disposed meanwhile goes nowhere', async () => {
  const run = runtime([call('slow', [])])
  await run.run(`
    let release
    const pending = new Promise((resolve) => { release = resolve })
    const first = await page.exposeFunction('slow', () => pending)
    await new Promise((resolve) => setTimeout(resolve, 10))
    await first.dispose()
    await page.exposeFunction('slow', () => 2)
    release(1)
    await new Promise((resolve) => setTimeout(resolve, 20))
  `)
  await settle()
  expect(run.sent('bindingCall.resolve')).toEqual([])
})

it('tests that a failed exposeFunctions setup disposes the bindings it made', async () => {
  let registered = 0
  const run = loadPrelude({
    seed: SEED,
    answer: (sent: Call) => {
      if (sent.name === 'events.next') return never
      if (sent.name === 'page.exposeBinding') return { id: ++registered }
      if (sent.name === 'page.addInitScript')
        return {
          err: { code: 'owner_has_control', owner: 'host', message: 'held' },
        }
      return {}
    },
  })
  const failed = await run.run(`
    try {
      await page.addInitScript(({ a, b }) => a(b()), { a: () => 1, b: () => 2 }, { exposeFunctions: true })
      return null
    } catch (error) {
      return error.code
    }
  `)
  expect(failed).toBe('owner_has_control')
  expect(
    run
      .sent('disposable.dispose')
      .map((sent) => sent.input.id)
      .sort(),
  ).toEqual([1, 2])
})

describe("evaluate's exposeFunctions", () => {
  const name = `__pw_fn_${'80'.repeat(16)}`

  it('tests that each function is exposed off window before the evaluate that names it, and answers the page', async () => {
    const run = runtime([call(name, [21])])
    await run.run(`
      Math.random = () => 0.5
      await page.evaluate(({ cb }) => cb(21), { cb: (n) => n * 2 }, { exposeFunctions: true })
      await new Promise((resolve) => setTimeout(resolve, 20))
    `)
    await settle()
    const ops = run.calls
      .map((sent) => sent.name)
      .filter((op) => op === 'page.exposeBinding' || op === 'page.evaluate')
    expect(ops).toEqual(['page.exposeBinding', 'page.evaluate'])
    expect(run.sent('page.exposeBinding').map((sent) => sent.input)).toEqual([
      {
        call: { api: 'page.evaluate', timeoutMs: 0 },
        page: { $neo: 'page', tab: 1007 },
        name,
        noGlobal: true,
      },
    ])
    const [evaluate] = run.sent('page.evaluate')
    const script = evaluate?.input.script as { run: { arg: string } }
    expect(JSON.parse(script.run.arg)).toEqual({
      o: [{ k: 'cb', v: { fn: name } }],
      id: 1,
    })
    const [answer] = run.sent('bindingCall.resolve')
    expect(JSON.parse(String(answer?.input.arg)).o[2]).toEqual({
      k: 'result',
      v: 42,
    })
  })

  it('tests that locator.evaluate exposes nothing when its check fails or its signal already fired', async () => {
    const run = runtime()
    const aborted = await run.run(`
      await page.locator('div').evaluate(() => 1, { cb: () => 1 }, { exposeFunctions: true, timeout: 5 })
      await page.locator('div').evaluate(() => 1, { cb: () => 1, page }, { exposeFunctions: true })
      const signal = { aborted: true, reason: 'stop', addEventListener() {}, removeEventListener() {} }
      return await page.locator('div').evaluate(() => 1, { cb: () => 1 }, { exposeFunctions: true, signal })
        .then(() => null, (error) => error.name)
    `)
    expect(aborted).toBe('AbortError')
    const scripts = run
      .sent('locator.evaluate')
      .map((sent) => sent.input.script as { refused?: string })
    expect(String(scripts[1]?.refused)).toStartWith(
      'Neo passes element handles',
    )
    expect(
      run
        .sent('page.exposeBinding')
        .map((sent) => (sent.input.call as { api: string }).api),
    ).toEqual(['locator.evaluate'])
  })

  it('tests that a failed callback setup disposes the callbacks that went in, and sends no evaluate', async () => {
    let registered = 0
    const run = loadPrelude({
      seed: SEED,
      answer: (sent: Call) => {
        if (sent.name === 'events.next') return never
        if (sent.name === 'page.exposeBinding')
          return ++registered === 1
            ? { id: 1 }
            : {
                err: {
                  code: 'owner_has_control',
                  owner: 'host',
                  message: 'held',
                },
              }
        return {}
      },
    })
    const failed = await run.run(`
      return await page.evaluate(({ a, b }) => a(b()), { a: () => 1, b: () => 2 }, { exposeFunctions: true })
        .then(() => null, (error) => error.code)
    `)
    expect(failed).toBe('owner_has_control')
    expect(run.sent('disposable.dispose').map((sent) => sent.input.id)).toEqual(
      [1],
    )
    expect(run.sent('page.evaluate')).toEqual([])
  })
})
