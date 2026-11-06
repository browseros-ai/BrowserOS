// Upstream: Playwright v1.64.1 tests/page/selectors-text.spec.ts:153
// Test: should work across nodes
// Lifted by tier2/convert.mjs. Rerun it instead of editing this file.
const tier2 = tier2Runtime(fixtures)
return tier2.done(await harness.settle((async () => {
  await page.goto(tier2.html(`<div id=target1>Hello<i>,</i> <span id=target2>world</span><b>!</b></div>`));

  (await tier2.record('L156', async () => (await page.$eval(`:text("Hello, world!")`, e => e.id))));
  (await tier2.record('L157', async () => (await page.$eval(`:text("Hello")`, e => e.id))));
  (await tier2.record('L158', async () => (await page.$eval(`:text("world")`, e => e.id))));
  (await tier2.record('L159', async () => (await page.$$eval(`:text("world")`, els => els.length))));
  (await tier2.record('L160', async () => (await page.$(`:text("hello world")`))));
  (await tier2.record('L161', async () => (await page.$(`div:text("world")`))));
  (await tier2.record('L162', async () => (await page.$eval(`text=Hello, world!`, e => e.id))));
  (await tier2.record('L163', async () => (await page.$eval(`text=Hello`, e => e.id))));
  (await tier2.record('L164', async () => (await page.$eval(`text=world`, e => e.id))));
  (await tier2.record('L165', async () => (await page.$$eval(`text=world`, els => els.length))));
  (await tier2.record('L166', async () => (await page.$(`text=hello world`))));

  (await tier2.record('L168', async () => (await page.$(`:text-is("Hello, world!")`))));
  (await tier2.record('L169', async () => (await page.$eval(`:text-is("Hello")`, e => e.id))));
  (await tier2.record('L170', async () => (await page.$eval(`:text-is("world")`, e => e.id))));
  (await tier2.record('L171', async () => (await page.$$eval(`:text-is("world")`, els => els.length))));
  (await tier2.record('L172', async () => (await page.$(`text="Hello, world!"`))));
  (await tier2.record('L173', async () => (await page.$eval(`text="Hello"`, e => e.id))));
  (await tier2.record('L174', async () => (await page.$eval(`text="world"`, e => e.id))));
  (await tier2.record('L175', async () => (await page.$$eval(`text="world"`, els => els.length))));

  (await tier2.record('L177', async () => (await page.$eval(`:text-matches(".*")`, e => e.nodeName))));
  (await tier2.record('L178', async () => (await page.$eval(`:text-matches("world?")`, e => e.id))));
  (await tier2.record('L179', async () => (await page.$$eval(`:text-matches("world")`, els => els.length))));
  (await tier2.record('L180', async () => (await page.$(`div:text(".*")`))));
  (await tier2.record('L181', async () => (await page.$eval(`text=/.*/`, e => e.nodeName))));
  (await tier2.record('L182', async () => (await page.$eval(`text=/world?/`, e => e.id))));
  (await tier2.record('L183', async () => (await page.$$eval(`text=/world/`, els => els.length))));
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
