// Upstream: Playwright v1.64.1 tests/page/selectors-css.spec.ts:180
// Test: should work with attribute selectors
// Lifted by tier2/convert.mjs. Rerun it instead of editing this file.
// @needs R10c: $eval (R10c) not built yet
const tier2 = tier2Runtime(fixtures)
return tier2.done(await harness.settle((async () => {
  await page.goto(tier2.html(`<div attr="hello world" attr2="hello-''>>foo=bar[]" attr3="] span"><span></span></div>`));
  await page.evaluate(() => (window)['div'] = document.querySelector('div'));
  const selectors = [
    `[attr="hello world"]`,
    `[attr = "hello world"]`,
    `[attr ~= world]`,
    `[attr ^=hello ]`,
    `[attr $= world ]`,
    `[attr *= "llo wor" ]`,
    `[attr2 |= hello]`,
    `[attr = "Hello World" i ]`,
    `[attr *= "llo WOR"i]`,
    `[attr $= woRLD i]`,
    `[attr2 = "hello-''>>foo=bar[]"]`,
    `[attr2 $="foo=bar[]"]`,
  ];
  for (const selector of selectors)
    (await tier2.record('L198', async () => (await page.$eval(selector, e => e === (window)['div']))));
  (await tier2.record('L199', async () => (await page.$eval(`[attr*=hello] span`, e => e.parentNode === (window)['div']))));
  (await tier2.record('L200', async () => (await page.$eval(`[attr*=hello] >> span`, e => e.parentNode === (window)['div']))));
  (await tier2.record('L201', async () => (await page.$eval(`[attr3="] span"] >> span`, e => e.parentNode === (window)['div']))));
})()))

function tier2Runtime(fixtures) {
  const expects = {}
  // A message's call log and stack frames are each runner's own (the page's V8, Node, QuickJS).
  const text = (value) =>
    value
      .split('\nCall log:')[0]
      .split('\n')
      .filter((line) => !/^\s+at /.test(line))
      .join('\n')
  const threw = (error) => ({
    threw: {
      name: String(error?.name ?? 'Error'),
      message: text(String(error?.message ?? error)),
    },
  })
  const shape = (value, depth) => {
    if (value === undefined) return '(undefined)'
    if (value === null || typeof value === 'boolean') return value
    if (typeof value === 'string') return text(value)
    if (typeof value === 'number') return Number.isFinite(value) ? value : String(value)
    if (typeof value === 'function') return '(function)'
    if (typeof value !== 'object') return String(value)
    if (depth > 8) return '(nested too deep)'
    if (Array.isArray(value)) return value.map((item) => shape(item, depth + 1))
    if (value instanceof Error) return threw(value)
    if (typeof value.asElement === 'function')
      return value.asElement() === null ? '(JSHandle)' : '(ElementHandle)'
    if (typeof value.evaluateAll === 'function') return '(Locator)'
    if (typeof value.mainFrame === 'function') return '(Page)'
    if (typeof value.parentFrame === 'function') return '(Frame)'
    if (ArrayBuffer.isView(value)) return `(${value.byteLength} bytes)`
    const shaped = {}
    for (const key of Object.keys(value)) shaped[key] = shape(value[key], depth + 1)
    return shaped
  }
  return {
    server: {
      PREFIX: fixtures.url('upstream', 'b'),
      CROSS_PROCESS_PREFIX: fixtures.url('upstream', 'a'),
      EMPTY_PAGE: fixtures.url('upstream/empty.html', 'b'),
    },
    html: (markup) => `data:text/html;charset=utf-8,${encodeURIComponent(markup)}`,
    /** toMatch's verdict: a string pattern is a substring, as in Jest. */
    matches: (value, pattern) =>
      typeof value !== 'string'
        ? shape(value, 0)
        : typeof pattern === 'string'
          ? value.includes(pattern)
          : pattern.test(value),
    /** Records what `received` gives (or throws) under the expect's line, as JSON. */
    record: (line, received) => settle(line, received, false),
    /** As record, then gives back what `received` gave, rejecting as it rejected. */
    pass: (line, received) => settle(line, received, true),
    done: (end) => ('threw' in end ? { ...expects, threw: threw(end.threw).threw } : expects),
  }

  function settle(line, received, passThrough) {
    let key = line
    for (let count = 2; key in expects; count++) key = `${line}#${count}`
    expects[key] = '(pending)'
    const put = (value) => {
      expects[key] = shape(value, 0)
      return value
    }
    const fail = (error) => {
      expects[key] = threw(error)
      if (passThrough) throw error
    }
    try {
      const value = received()
      return typeof value?.then === 'function' ? value.then(put, fail) : put(value)
    } catch (error) {
      return fail(error)
    }
  }
}
