// page.evaluate's argument as Playwright's client and server carry it into the page: the page
// describes what it received ([typeof, Object.prototype.toString.call, String], item by item),
// so the case sees the value the page saw. Then the client's own checks: a function in the
// argument, too many arguments, a third argument that is not an object, and the channel
// validator's refusal of an invalid Date; and the server's: a function that is not
// well-serializable. Method shorthands and expressions are normalized. Element handles pass by
// reference: one of the page's document, the same one twice, one inside an object; one of a
// same-origin frame and one of a cross-origin frame into the main frame; and a disposed one.
// A waitForFunction handle passes its value: the same one twice is one object in the page; and
// whether it is the page's own object, sees the page's later change, and passes once disposed.
await page.goto(fixtures.url('reads.html'))
const received = await page.evaluate((arg) => {
  const describe = (value, seen = new Set()) => {
    if (value !== null && typeof value === 'object') {
      if (seen.has(value)) return 'cycle'
      seen.add(value)
    }
    const tag = Object.prototype.toString.call(value)
    const text = Object.is(value, -0)
      ? '-0'
      : tag === '[object Date]'
        ? Number.isNaN(value.getTime())
          ? 'Invalid Date'
          : value.toISOString()
        : String(value)
    if (Array.isArray(value))
      return [typeof value, tag, value.map((item) => describe(item, seen))]
    if (tag === '[object Object]')
      return [
        typeof value,
        tag,
        Object.entries(value).map(([key, item]) => [key, describe(item, seen)]),
      ]
    return [typeof value, tag, text]
  }
  return arg.map((item) => describe(item))
}, [
  undefined,
  null,
  NaN,
  -0,
  Infinity,
  10n,
  true,
  'text',
  new Date(0),
  /a+b/gi,
  Object.assign(new TypeError('no'), { stack: 'TypeError: no\n    at script' }),
  Uint8Array.of(1, 2, 255),
  Float64Array.of(1.5),
  new ArrayBuffer(2),
  new Map([[1, 2]]),
  Symbol('s'),
  { nested: { deep: [1, { x: 'y' }] } },
])
const heading = await page.$('h1')
const cycle = { n: 1 }
cycle.self = cycle
const shared = { y: 1 }
const keep = (work) => harness.settle(work)
return {
  received,
  cycle: await page.evaluate((arg) => arg.self === arg && arg.n, cycle),
  shared: await page.evaluate((arg) => arg[0] === arg[1], [shared, shared]),
  undefinedArg: await page.evaluate((arg) => arg === undefined),
  expressionWithArg: await page.evaluate('6 * 7', 5),
  number: await page.evaluate(42),
  shorthand: await page.evaluate({ answer() { return 7 } }.answer),
  asyncShorthand: await page.evaluate({ async later() { return 8 } }.later),
  functionArg: await keep(page.evaluate((x) => x, () => 1)),
  nestedFunction: await keep(page.evaluate((x) => x, { a: { f: () => 1 } })),
  functionInArray: await keep(page.evaluate((x) => x, [1, () => 1])),
  toJSONMethod: await keep(page.evaluate((x) => x, { toJSON() { return 'json' } })),
  invalidDate: await keep(page.evaluate((x) => x, [1, { when: new Date('not a date') }])),
  tooMany: await keep(page.evaluate(() => 1, 1, {}, 4)),
  optionsNotObject: await keep(page.evaluate(() => 1, 1, 2)),
  optionsArray: await keep(page.evaluate(() => 1, 1, [])),
  notSerializable: await keep(
    page.evaluate(Object.getOwnPropertyDescriptor({ get x() { return 1 } }, 'x').get),
  ),
  handle: await keep(page.evaluate((el) => el.tagName, heading)),
  handleTwice: await keep(page.evaluate(([a, b]) => a === b, [heading, heading])),
  handleInObject: await keep(page.evaluate(({ el }) => el.isConnected, { el: heading })),
  handleOnElement: await keep(page.locator('h1').evaluate((el, other) => el === other, heading)),
  waitHandle: await keep(
    page.waitForFunction(() => ({ ready: 42 })).then((ready) => page.evaluate((x) => [typeof x, x.ready], ready)),
  ),
  waitHandleTwice: await keep(
    page.waitForFunction(() => ({ ready: 42 })).then((ready) => page.evaluate(([a, b]) => a === b, [ready, ready])),
  ),
  waitHandleByReference: await keep(
    (async () => {
      const held = await page.waitForFunction(() => (window.held = { n: 1 }))
      await harness.pageEval(page, 'window.held.n = 2')
      return await page.evaluate((x) => `own object: ${x === window.held}, n: ${x.n}`, held)
    })(),
  ),
  waitHandleDisposed: await keep(
    (async () => {
      const held = await page.waitForFunction(() => ({ n: 1 }))
      await held.dispose()
      return await page.evaluate((x) => x.n, held)
    })().catch((error) => {
      throw Object.assign(new Error(error.message.replace(/handle@[0-9a-f]+/, 'handle@<guid>')), { name: error.name })
    }),
  ),
  ...(await (async () => {
    await page.goto(fixtures.url('frames.html'))
    await harness.poll(page, "!!document.querySelector('iframe[name=same]').contentDocument?.querySelector('#name')")
    const same = await page.frame({ name: 'same' }).$('#name')
    const cross = await page.frame({ name: 'cross' }).$('#name')
    const disposed = await page.$('h1')
    await disposed.dispose()
    const guid = (outcome) =>
      outcome.threw
        ? { threw: { ...outcome.threw, message: outcome.threw.message.replace(/handle@[0-9a-f]+/, 'handle@<guid>') } }
        : outcome
    return {
      sameOriginFrame: await keep(page.evaluate((el) => el.textContent, same)),
      crossOriginFrame: await keep(page.evaluate((el) => el.textContent, cross)),
      inItsOwnFrame: await keep(page.frame({ name: 'cross' }).locator('#name').evaluate((el, other) => el === other, cross)),
      disposedArg: guid(await keep(page.evaluate((el) => el, disposed))),
      disposedCall: await keep(disposed.evaluate((el) => el)),
    }
  })()),
}
