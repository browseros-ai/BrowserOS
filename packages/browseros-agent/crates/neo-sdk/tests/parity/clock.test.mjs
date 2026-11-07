// page.clock (the context's clock) through the Playwright reference and through Neo: install
// fakes the page's timers and `Date`, runFor and fastForward move a paused clock (timers fire or
// are skipped as Playwright's do), setSystemTime and setFixedTime set what `Date` reads, pauseAt
// and resume stop and restart time, a new document replays the clock, and Playwright's errors
// for a tick count or a time it cannot read. Timers report through an exposed `stub`, so each
// engine runs in a browser of its own (`isolate`, as for exposeFunction.test.mjs). Values by
// `describe` (values.mjs), errors by category and the words Playwright's tests look for.
//
// Neo's clock ends with the run, every document getting its own timers back (`Deviation:` in
// clock-call.ts); `neo-runtime/tests/clock_e2e.rs` shows it.
//
// Cases from tests/library/page-clock.spec.ts at 26137face: CK-001 "triggers immediately without
// specified delay", CK-002 "does not trigger without sufficient delay", CK-003 "triggers after
// sufficient delay", CK-012 "throws for invalid format", CK-014 "ignores timers which wouldn't be
// run", CK-016 "supports string time arguments", CK-017 "sets initial timestamp", CK-018 "should
// throw for invalid date", CK-031 "does not fake methods", CK-032 "allows setting time multiple
// times", CK-035 "should progress time", CK-038 "should pause".
import assert from 'node:assert/strict'
import { test } from 'node:test'
import { compareMethod, skip } from './harness.mjs'
import { VALUES } from './values.mjs'

const html = '<title>clock</title><p id="p">text</p>'

// Playwright's fixture: the clock installed at 0 and paused at 1000, timers calling `stub`.
const PAUSED = `
  const calls = []
  await page.exposeFunction('stub', (...params) => { calls.push({ params }) })
  await page.clock.install({ time: 0 })
  await page.clock.pauseAt(1000)
`

test('runFor runs the timers it reaches, and refuses what it cannot read', {
  skip,
  timeout: 180_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html,
    source: `${VALUES}${PAUSED}
      await page.evaluate(() => { setTimeout(window.stub) })
      await page.clock.runFor(0)
      const immediately = calls.length
      await page.evaluate(() => { setTimeout(window.stub, 100) })
      await page.clock.runFor(10)
      const tooSoon = calls.length
      await page.clock.runFor(90)
      const reached = calls.length
      const format = await outcome(() => page.clock.runFor('12:02:34:10'), ['Clock only understands'])
      const nan = await outcome(() => page.clock.runFor(NaN), ['clock.runFor: Invalid ticks: NaN'])
      const infinity = await outcome(() => page.clock.fastForward(Infinity), ['clock.fastForward: Invalid ticks: Infinity'])
      await page.goto(page.url())
      const replayed = await page.evaluate(() => Date.now())
      return { immediately, tooSoon, reached, format, nan, infinity, replayed }
    `,
    isolate: true,
    probe: '({ title: document.title })',
  })
  assert.deepEqual(
    {
      ...outcome.value,
      format: undefined,
      nan: undefined,
      infinity: undefined,
    },
    {
      immediately: 1,
      tooSoon: 1,
      reached: 2,
      format: undefined,
      nan: undefined,
      infinity: undefined,
      replayed: 1100,
    },
  )
})

test('fastForward skips the timers it passes, and reads mm:ss', {
  skip,
  timeout: 180_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html,
    source: `${PAUSED}
      await page.evaluate(() => { setTimeout(() => window.stub('should not be logged'), 1000) })
      await page.clock.fastForward(500)
      const skipped = calls.length
      await page.evaluate(() => { setTimeout(() => window.stub(Date.now()), 100000) })
      await page.clock.fastForward('01:50')
      return { skipped, calls }
    `,
    isolate: true,
    probe: '({ title: document.title })',
  })
  assert.equal(outcome.value.skipped, 0)
})

test('setSystemTime and setFixedTime set what Date reads', {
  skip,
  timeout: 180_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html,
    source: `${VALUES}${PAUSED}
      await page.clock.setSystemTime(1400)
      const system = await page.evaluate(() => Date.now())
      const invalidDate = await outcome(() => page.clock.setSystemTime(new Date('invalid')), ['Invalid date: Invalid Date'])
      const invalidString = await outcome(() => page.clock.setSystemTime('invalid'), ['clock.setSystemTime: Invalid date: invalid'])
      await page.clock.setFixedTime(100)
      const fixed = await page.evaluate(() => Date.now())
      await page.clock.setFixedTime(200)
      const again = await page.evaluate(() => Date.now())
      return { system, invalidDate, invalidString, fixed, again }
    `,
    isolate: true,
    probe: '({ title: document.title })',
  })
  assert.equal(outcome.value.system, 1400)
  assert.equal(outcome.value.fixed, 100)
  assert.equal(outcome.value.again, 200)
})

test('a fixed time fakes no timers', {
  skip,
  timeout: 180_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html,
    source: `
      await page.clock.setFixedTime(100)
      const now = await page.evaluate(() => Date.now())
      const timer = await page.evaluate(() => new Promise((resolve) => setTimeout(() => resolve('fired'), 1)))
      return { now, timer }
    `,
    isolate: true,
    probe: '({ title: document.title })',
  })
  assert.deepEqual(outcome.value, { now: 100, timer: 'fired' })
})

test('pauseAt stops time and resume restarts it', {
  skip,
  timeout: 180_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html,
    source: `
      await page.clock.install({ time: 0 })
      await page.goto(page.url())
      await page.clock.pauseAt(1000)
      const paused = [await page.evaluate(() => Date.now())]
      await new Promise((resolve) => setTimeout(resolve, 200))
      paused.push(await page.evaluate(() => Date.now()))
      await page.clock.resume()
      await new Promise((resolve) => setTimeout(resolve, 200))
      const resumed = (await page.evaluate(() => Date.now())) > 1000
      return { paused, resumed }
    `,
    isolate: true,
    probe: '({ title: document.title })',
  })
  assert.deepEqual(outcome.value, { paused: [1000, 1000], resumed: true })
})
