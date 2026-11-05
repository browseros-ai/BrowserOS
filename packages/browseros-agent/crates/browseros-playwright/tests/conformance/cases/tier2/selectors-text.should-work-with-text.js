// Upstream: Playwright v1.64.1 tests/page/selectors-text.spec.ts:118
// Test: should work with :text
// Lifted by tier2/convert.mjs. Rerun it instead of editing this file.
// @needs R10c: $eval (R10c) not built yet
const tier2 = tier2Runtime(fixtures)
return tier2.done(await harness.settle((async () => {
  await page.goto(tier2.html(`<div>yo</div><div>ya</div><div>\nHELLO   \n world  </div>`));
  (await tier2.record('L120', async () => (await page.$eval(`:text("ya")`, e => e.outerHTML))));
  (await tier2.record('L121', async () => (await page.$eval(`:text-is("ya")`, e => e.outerHTML))));
  (await tier2.record('L122', async () => (await page.$eval(`:text("y")`, e => e.outerHTML))));
  (await tier2.record('L123', async () => (await page.$(`:text-is("Y")`))));
  (await tier2.record('L124', async () => (await page.$eval(`:text("hello world")`, e => e.outerHTML))));
  (await tier2.record('L125', async () => (await page.$eval(`:text-is("HELLO world")`, e => e.outerHTML))));
  (await tier2.record('L126', async () => (await page.$eval(`:text("lo wo")`, e => e.outerHTML))));
  (await tier2.record('L127', async () => (await page.$(`:text-is("lo wo")`))));
  (await tier2.record('L128', async () => (await page.$eval(`:text-matches("^[ay]+$")`, e => e.outerHTML))));
  (await tier2.record('L129', async () => (await page.$eval(`:text-matches("y", "g")`, e => e.outerHTML))));
  (await tier2.record('L130', async () => (await page.$eval(`:text-matches("Y", "i")`, e => e.outerHTML))));
  (await tier2.record('L131', async () => (await page.$(`:text-matches("^y$")`))));

  const error1 = await page.$(`:text("foo", "bar")`).catch(e => e);
  tier2.record('L134', () => (error1.message));
  const error2 = await page.$(`:text(foo > bar)`).catch(e => e);
  tier2.record('L136', () => (error2.message));
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
