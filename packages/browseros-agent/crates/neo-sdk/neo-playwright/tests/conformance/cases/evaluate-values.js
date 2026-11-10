// page.evaluate's results as Playwright's serializer carries them out of the page: every kind
// of value, each described [typeof, Object.prototype.toString.call, String] because an outcome
// is JSON, with arrays and plain objects described item by item. Cycles and shared objects keep
// their identity; nodes, functions and symbols come back as upstream's serializer writes them.
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
await page.goto(fixtures.url('reads.html'))
const values = await page.evaluate(() => [
  undefined,
  null,
  NaN,
  -0,
  Infinity,
  -Infinity,
  10n,
  -5n,
  true,
  42,
  1.5,
  'text',
  new Date(0),
  new Date('not a date'),
  /a+b/gi,
  new TypeError('no'),
  Uint8Array.of(1, 2, 255),
  Float64Array.of(1.5),
  BigInt64Array.of(5n),
  new ArrayBuffer(2),
  new Map([[1, 2]]),
  new Set([1]),
  Symbol('s'),
  () => 1,
  [() => 1, 2],
  { f: () => 1, n: 2 },
  { toJSON() { return 1 }, n: 2 },
  Object.create({ toJSON() { return 'json' } }),
  JSON.parse('{"__proto__": 1, "n": 2}'),
  document.body,
  document,
  window,
  [[[]], {}],
])
const cycle = await page.evaluate(() => {
  const o = { n: 1 }
  o.self = o
  return o
})
const shared = await page.evaluate(() => {
  const x = { y: 1 }
  return [x, x]
})
const url = await page.evaluate(() => new URL('https://a.example/p?q=1#h'))
return {
  values: values.map((value) => describe(value)),
  cycle: [describe(cycle), cycle.self === cycle],
  shared: [describe(shared), shared[0] === shared[1]],
  nothing: describe(await page.evaluate(() => undefined)),
  fn: describe(await page.evaluate(() => () => 1)),
  promised: describe(await page.evaluate(async () => new Date(5))),
  expression: describe(await page.evaluate('[document.title, 6 * 7]')),
  url: [typeof url, Object.prototype.toString.call(url), String(url), url.hostname, url.search],
}
