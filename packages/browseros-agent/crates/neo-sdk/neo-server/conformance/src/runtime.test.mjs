import { expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import vm from 'node:vm'
import { expect as pwExpect } from '@playwright/test'

const create = vm.runInThisContext(
  readFileSync(new URL('./runtime.js', import.meta.url), 'utf8'),
)
function harness(config = {}) {
  return create({
    context: { pages: () => [], newPage: async () => ({}) },
    expect: pwExpect,
    setTimeout,
    clearTimeout,
    now: Date.now,
    config: { fixtures: {}, testTimeout: 50, expectTimeout: 10000, ...config },
  })
}
test('hook errors fail, retaining the first error and running teardown', async () => {
  const h = harness(),
    order = []
  h.test.beforeEach(() => {
    order.push('before')
    throw new Error('first')
  })
  h.test.afterEach(() => {
    order.push('after')
    throw new Error('second')
  })
  h.test.afterAll(() => {
    order.push('all')
    throw new Error('third')
  })
  h.test('test', () => order.push('body'))
  const r = await h.runOne(0)
  expect(r.status).toBe('failed')
  expect(r.errors.map((e) => e.message)).toEqual(['first', 'second', 'third'])
  expect(order).toEqual(['before', 'after', 'all'])
})
test('timeout still runs afterEach and afterAll', async () => {
  const h = harness({ testTimeout: 10 }),
    order = []
  h.test.afterEach(() => order.push('after'))
  h.test.afterAll(() => order.push('all'))
  h.test('hang', () => new Promise(() => {}))
  expect((await h.runOne(0)).status).toBe('timedOut')
  expect(order).toEqual(['after', 'all'])
})
test('beforeAll rejects page/context fixtures', async () => {
  for (const name of ['page', 'context']) {
    const h = harness()
    h.test.beforeAll(new Function(`return async ({${name}})=>{}`)())
    h.test('test', () => {})
    expect((await h.runOne(0)).errors[0].message).toContain(
      'fixtures are not supported',
    )
  }
})
test('soft errors and runtime skip/fail have correct status', async () => {
  const soft = harness()
  soft.test('soft', () => {
    soft.expect.soft(1).toBe(2)
    soft.expect(2).toBe(2)
  })
  const s = await soft.runOne(0)
  expect(s.status).toBe('failed')
  expect(s.softErrors.length).toBe(1)
  const skip = harness()
  skip.test('skip', () => skip.test.skip(true, 'reason'))
  expect((await skip.runOne(0)).status).toBe('skipped')
  const fail = harness()
  fail.test.fail.only('fail', () => {
    throw new Error('expected')
  })
  expect((await fail.runOne(0)).expectedStatus).toBe('failed')
})
test('testInfo and test.setTimeout reschedule the active deadline', async () => {
  const h = harness({ testTimeout: 10 })
  h.test('extend', async (_, info) => {
    info.setTimeout(100)
    await new Promise((r) => setTimeout(r, 20))
    h.test.setTimeout(100)
  })
  expect((await h.runOne(0)).status).toBe('passed')
})
test('original matcher binding, statics, custom matching and retry counts', async () => {
  const h = harness()
  h.expect.extend({
    even: (n) => ({ pass: n % 2 === 0, message: () => 'odd' }),
  })
  h.test('expect', async () => {
    h.expect({ x: 1 }).toEqual(h.expect.objectContaining({ x: 1 }))
    h.expect(2).even()
    h.expect(3).not.even()
    await h.expect(Promise.resolve(4)).resolves.toBe(4)
    let attempts = 0
    await h
      .expect(() => {
        h.expect(++attempts).toBeGreaterThan(1)
      })
      .toPass({ intervals: [1], timeout: 100 })
    await h.expect
      .poll(() => ++attempts, { intervals: [1], timeout: 100 })
      .toBeGreaterThan(2)
  })
  const r = await h.runOne(0)
  expect(r.status).toBe('passed')
  expect(r.assertions.called).toBe(6)
})
test('error text differs softly but missing errors and page text fail', async () => {
  const h = harness()
  h.test('diagnostic', () => {
    h.messageExpect(
      { kind: 'message', line: 1 },
      new Error('actual'),
    ).toContain('expected')
    h.messageExpect({ kind: 'throw', line: 2 }, () => {
      throw new Error('actual')
    }).toThrow('expected')
  })
  const r = await h.runOne(0)
  expect(r.status).toBe('passed')
  expect(r.messageDiffs.length).toBe(2)
  const missing = harness()
  missing.test('missing', () =>
    missing.messageExpect({ kind: 'throw' }, () => {}).toThrow('x'),
  )
  expect((await missing.runOne(0)).status).toBe('failed')
  const page = harness()
  page.test('text', () => page.expect('page text').toBe('different'))
  expect((await page.runOne(0)).status).toBe('failed')
})
test('runtime skips still tear down, static skips do not run hooks', async () => {
  const h = harness(),
    order = []
  h.test.afterEach(() => order.push('after'))
  h.test('runtime', () => h.test.skip(true, 'runtime'))
  expect((await h.runOne(0)).status).toBe('skipped')
  expect(order).toEqual(['after'])
  const s = harness(),
    staticOrder = []
  s.test.beforeAll(() => staticOrder.push('before'))
  s.test.afterAll(() => staticOrder.push('after'))
  s.test.skip('static', () => {})
  expect((await s.runOne(0)).status).toBe('skipped')
  expect(staticOrder).toEqual([])
})
test('beforeAll timeout changes are scoped to the hook', async () => {
  const h = harness({ testTimeout: 10 })
  h.test.beforeAll(async (_, info) => {
    info.setTimeout(80)
    await new Promise((r) => setTimeout(r, 20))
  })
  h.test('body', () => new Promise(() => {}))
  expect((await h.runOne(0)).status).toBe('timedOut')
})

test('diagnostic throw wording captures one invocation and rejects a no-throw result', async () => {
  for (const wording of ['expected', 'different']) {
    const h = harness()
    let calls = 0
    h.test('single invocation', () =>
      h
        .messageExpect({ kind: 'throw', line: 1 }, () => {
          if (++calls === 1) throw new TypeError(wording)
        })
        .toThrow('expected'),
    )
    const r = await h.runOne(0)
    expect(calls).toBe(1)
    expect(r.status).toBe('passed')
    expect(r.diagnostics[0]).toMatchObject({
      matched: wording === 'expected',
      error: { name: 'TypeError', message: wording },
    })
  }
  const h = harness()
  let calls = 0
  h.test('no throw', () =>
    h
      .messageExpect({ kind: 'throw' }, () => {
        calls++
      })
      .toThrow('expected'),
  )
  expect((await h.runOne(0)).status).toBe('failed')
  expect(calls).toBe(1)
})

test('diagnostic rejects uses the original rejection', async () => {
  const h = harness()
  h.test('rejects', async () => {
    await h
      .messageExpect(
        { kind: 'throw', rejects: true },
        Promise.reject(new Error('actual')),
      )
      .rejects.toThrow('expected')
  })
  const r = await h.runOne(0)
  expect(r.status).toBe('passed')
  expect(r.diagnostics[0].error.message).toBe('actual')
})
