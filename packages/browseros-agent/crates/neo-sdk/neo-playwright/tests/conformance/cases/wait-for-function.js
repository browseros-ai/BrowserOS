// page.waitForFunction: a predicate evaluated once and called on every poll, so one that keeps
// state in its argument resolves; polling by interval; the client's and the server's polling
// checks; a timeout and a throw; and the handle it resolves to, for values of every kind:
// jsonValue() described [typeof, Object.prototype.toString.call, String], toString() and
// asElement(). The default 'raf' polling is timed, not compared by value: five polls take
// five animation frames upstream, and five 100 ms steps in Neo.
const describe = (value) => {
  const tag = Object.prototype.toString.call(value)
  if (Array.isArray(value)) return [typeof value, tag, value.map(describe)]
  if (tag === '[object Object]')
    return [typeof value, tag, Object.entries(value).map(([key, item]) => [key, describe(item)])]
  if (tag === '[object Date]') return [typeof value, tag, value.toISOString()]
  return [typeof value, tag, String(value)]
}
await page.goto(fixtures.url('reads.html'))
const keep = (work) => harness.settle(work)
const handled = async (predicate) => {
  const handle = await page.waitForFunction(predicate)
  const shape = {
    value: describe(await handle.jsonValue()),
    text: handle.toString(),
    element: handle.asElement(),
  }
  await handle.dispose()
  return shape
}
await harness.pageEval(page, 'window.polls = 0; window.calls = 0')
return {
  stateful: await keep(
    page
      .waitForFunction((state) => ++state.polls >= 3 && state.polls, { polls: 0 })
      .then((handle) => handle.jsonValue()),
  ),
  interval: await keep(
    page
      .waitForFunction(() => ++window.polls >= 3 && window.polls, undefined, { polling: 50 })
      .then((handle) => handle.jsonValue()),
  ),
  expression: await keep(
    page
      .waitForFunction('++window.calls >= 2 && window.calls')
      .then((handle) => handle.jsonValue()),
  ),
  unknownPolling: await keep(page.waitForFunction(() => true, undefined, { polling: 'often' })),
  zeroInterval: await keep(page.waitForFunction(() => true, undefined, { polling: 0 })),
  negativeInterval: await keep(page.waitForFunction(() => true, undefined, { polling: -5 })),
  timedOut: await keep(page.waitForFunction(() => false, undefined, { timeout: 300 })),
  thrown: await keep(page.waitForFunction(() => { throw new Error('no') })),
  rafPolling: await keep(
    (async () => {
      await harness.pageEval(page, 'window.frames5 = 0')
      const started = Date.now()
      await page.waitForFunction(() => ++window.frames5 >= 5)
      return { withinFiveFrames: Date.now() - started < 300 }
    })(),
  ),
  asyncFalsy: await keep(
    page.waitForFunction(async () => 0).then((handle) => handle.jsonValue()),
  ),
  handles: {
    number: await handled(() => 42),
    string: await handled(() => 'ready'),
    object: await handled(() => ({ a: 1, b: [2] })),
    array: await handled(() => [1, 2]),
    bigint: await handled(() => 10n),
    date: await handled(() => new Date(0)),
    regexp: await handled(() => /x/g),
    infinity: await handled(() => Infinity),
  },
}
