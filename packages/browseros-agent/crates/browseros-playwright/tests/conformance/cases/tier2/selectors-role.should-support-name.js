// Upstream: Playwright v1.64.1 tests/page/selectors-role.spec.ts:436
// Test: should support name
// Lifted by tier2/convert.mjs. Rerun it instead of editing this file.
// @needs R10c: evaluateAll (R10c) not built yet
const tier2 = tier2Runtime(fixtures)
return tier2.done(await harness.settle((async () => {
  await page.goto(tier2.html(`
    <div role="button" aria-label=" Hello "></div>
    <div role="button" aria-label="Hallo"></div>
    <div role="button" aria-label="Hello" aria-hidden="true"></div>
    <div role="button" aria-label="123" aria-hidden="true"></div>
    <div role="button" aria-label='foo"bar' aria-hidden="true"></div>
  `));
  (await tier2.record('L444', async () => (await page.locator(`role=button[name="Hello"]`).evaluateAll(els => els.map(e => e.outerHTML)))));
  (await tier2.record('L447', async () => (await page.locator(`role=button[name=" \n Hello "]`).evaluateAll(els => els.map(e => e.outerHTML)))));
  (await tier2.record('L450', async () => (await page.getByRole('button', { name: 'Hello' }).evaluateAll(els => els.map(e => e.outerHTML)))));

  (await tier2.record('L454', async () => (await page.locator(`role=button[name*="all"]`).evaluateAll(els => els.map(e => e.outerHTML)))));

  (await tier2.record('L458', async () => (await page.locator(`role=button[name=/^H[ae]llo$/]`).evaluateAll(els => els.map(e => e.outerHTML)))));
  (await tier2.record('L462', async () => (await page.getByRole('button', { name: /^H[ae]llo$/ }).evaluateAll(els => els.map(e => e.outerHTML)))));

  (await tier2.record('L467', async () => (await page.locator(`role=button[name=/h.*o/i]`).evaluateAll(els => els.map(e => e.outerHTML)))));
  (await tier2.record('L471', async () => (await page.getByRole('button', { name: /h.*o/i }).evaluateAll(els => els.map(e => e.outerHTML)))));

  (await tier2.record('L476', async () => (await page.locator(`role=button[name="Hello"][include-hidden]`).evaluateAll(els => els.map(e => e.outerHTML)))));
  (await tier2.record('L480', async () => (await page.getByRole('button', { name: 'Hello', includeHidden: true }).evaluateAll(els => els.map(e => e.outerHTML)))));
  (await tier2.record('L484', async () => (await page.getByRole('button', { name: 'hello', includeHidden: true }).evaluateAll(els => els.map(e => e.outerHTML)))));

  (await tier2.record('L489', async () => (await page.locator(`role=button[name=Hello]`).evaluateAll(els => els.map(e => e.outerHTML)))));
  (await tier2.record('L492', async () => (await page.locator(`role=button[name=123][include-hidden]`).evaluateAll(els => els.map(e => e.outerHTML)))));
  (await tier2.record('L495', async () => (await page.getByRole('button', { name: '123', includeHidden: true }).evaluateAll(els => els.map(e => e.outerHTML)))));
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
