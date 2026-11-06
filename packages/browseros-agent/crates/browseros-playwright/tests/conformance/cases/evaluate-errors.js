// How page.evaluate fails, as Playwright prints it (diff.mjs compares first lines: the page's
// stack lines below name each runner's own in-page function). A thrown Error, primitive and
// object, a rejected promise, a syntax error, and results nested ever deeper: 99 nested objects
// are the deepest a CDP message carries, so 100 fail. A reload under a pending script is not
// here: real Playwright's call never settles on BrowserOS (measured), so nothing can compare.
// A function named __pw_fn_… comes back as a binding's caller, which upstream's server cannot
// serialize again, at the top or deep in the value.
await page.goto(fixtures.url('reads.html'))
const keep = (work) => harness.settle(work)
const nested = (depth) =>
  `(() => { let v = 0; for (let i = 0; i < ${depth}; i++) v = { a: v }; return v })()`
const depthOf = (value) => {
  let depth = 0
  for (let level = value; typeof level === 'object'; level = level.a) depth++
  return depth
}
return {
  shallow: await keep(page.evaluate(nested(10)).then(depthOf)),
  deepest: await keep(page.evaluate(nested(99)).then(depthOf)),
  error: await keep(page.evaluate(() => { throw new TypeError('bad thing') })),
  rejected: await keep(page.evaluate(async () => { throw new Error('later') })),
  string: await keep(page.evaluate(() => { throw 'plain' })),
  object: await keep(page.evaluate(() => { throw { a: 1 } })),
  nullThrown: await keep(page.evaluate(() => { throw null })),
  syntax: await keep(page.evaluate('this is not javascript')),
  tooDeep: await keep(page.evaluate(nested(100))),
  binding: await keep(page.evaluate(() => { function __pw_fn_demo() {} return __pw_fn_demo })),
  bindingOnElement: await keep(
    page.locator('h1').evaluate(() => { function __pw_fn_demo() {} return [__pw_fn_demo] }),
  ),
  bindingDeep: await keep(
    page.evaluate(() => { function __pw_fn_demo() {} return [1, { f: [__pw_fn_demo] }] }),
  ),
}
