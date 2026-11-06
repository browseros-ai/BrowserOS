// Upstream: Playwright v1.64.1 tests/page/selectors-css.spec.ts:406
// Test: should work with :scope
// Lifted by tier2/convert.mjs. Rerun it instead of editing this file.
const tier2 = tier2Runtime(fixtures)
const server = tier2.server
return tier2.done(await harness.settle((async () => {
  await page.goto(server.PREFIX + '/deep-shadow.html');
  // 'is' does not change the scope, so it remains 'html'.
  (await tier2.record('L409', async () => (await page.$$eval(`css=div:is(:scope#root1)`, els => els.length))));
  (await tier2.record('L410', async () => (await page.$$eval(`css=div:is(:scope #root1)`, els => els.length))));
  // 'has' does change the scope, so it becomes the 'div' we are querying.
  (await tier2.record('L412', async () => (await page.$$eval(`css=div:has(:scope > #target)`, els => els.length))));

  const handle = await page.$(`css=span`);
  for (const scope of [page, handle]) {
    (await tier2.record('L416', async () => (await scope.$$eval(`css=:scope`, els => els.length))));
    (await tier2.record('L417', async () => (await scope.$$eval(`css=* :scope`, els => els.length))));
    (await tier2.record('L418', async () => (await scope.$$eval(`css=* + :scope`, els => els.length))));
    (await tier2.record('L419', async () => (await scope.$$eval(`css=* > :scope`, els => els.length))));
    (await tier2.record('L420', async () => (await scope.$$eval(`css=* ~ :scope`, els => els.length))));
  }

  await page.goto(tier2.html(`<article><div class=target>hello<span></span></div></article>`));
  // 'scope' should allow additional native css modifiers
  (await tier2.record('L425', async () => (await page.$eval(`div >> :scope.target`, e => e.textContent))));
  (await tier2.record('L426', async () => (await page.$eval(`div >> :scope:nth-child(1)`, e => e.textContent))));
  (await tier2.record('L427', async () => (await page.$eval(`div >> :scope.target:has(span)`, e => e.textContent))));
  (await tier2.record('L428', async () => (await page.$eval(`html:scope`, e => e.nodeName))));

  await page.goto(tier2.html(`<section><span id=span1><span id=inner></span></span><span id=span2></span></section>`));
  (await tier2.record('L431', async () => (await page.$$eval(`#span1 >> span:not(:has(:scope > div))`, els => els.map(e => e.id)))));
  (await tier2.record('L432', async () => (await page.$$eval(`#span1 >> #inner,:scope`, els => els.map(e => e.id)))));
  (await tier2.record('L433', async () => (await page.$$eval(`#span1 >> span,:scope`, els => els.map(e => e.id)))));
  (await tier2.record('L434', async () => (await page.$$eval(`#span1 >> span:not(:scope)`, els => els.map(e => e.id)))));
  // TODO: the following two do not work. We do not expand the context for the inner :scope,
  // because we should only expand for one clause of :is() that contains :scope, but not the other.
  // expect(await page.$$eval(`#span1 >> span:is(:scope)`, els => els.map(e => e.id))).toEqual(['span1']);
  // expect(await page.$$eval(`#span1 >> span:is(:scope,#inner)`, els => els.map(e => e.id))).toEqual(['span1', 'inner']);
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
