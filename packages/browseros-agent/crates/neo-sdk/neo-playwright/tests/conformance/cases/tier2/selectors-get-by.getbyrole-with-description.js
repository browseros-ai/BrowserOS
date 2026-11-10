// Upstream: Playwright v1.64.1 tests/page/selectors-get-by.spec.ts:297
// Test: getByRole with description
// Lifted by tier2/convert.mjs. Rerun it instead of editing this file.
const tier2 = tier2Runtime(fixtures)
return tier2.done(await harness.settle((async () => {
  await page.goto(tier2.html(`
    <div role="alert" aria-label="Upload successful" aria-description="File doc-2025.pdf was uploaded successfully">Alert 1</div>
    <div role="alert" aria-label="Upload successful" aria-description="File report-2026.pdf was uploaded successfully">Alert 2</div>
    <div role="alert" aria-label="Invalid file" aria-description="File demo.doc has an invalid file format">Alert 3</div>
  `));

  // description substring match (default)
  (await tier2.record('L305', async () => (await page.getByRole('alert', { description: 'doc-2025' }).evaluateAll(els => els.map(e => e.textContent)))));
  (await tier2.record('L308', async () => (await page.getByRole('alert', { description: 'report-2026' }).evaluateAll(els => els.map(e => e.textContent)))));

  // description with name combined
  (await tier2.record('L313', async () => (await page.getByRole('alert', { name: 'Upload successful', description: 'doc-2025' }).evaluateAll(els => els.map(e => e.textContent)))));
  (await tier2.record('L316', async () => (await page.getByRole('alert', { name: 'Upload successful', description: 'report-2026' }).evaluateAll(els => els.map(e => e.textContent)))));
  (await tier2.record('L319', async () => (await page.getByRole('alert', { name: 'Invalid file', description: 'doc-2025' }).evaluateAll(els => els.map(e => e.textContent)))));

  // description exact match
  (await tier2.record('L323', async () => (await page.getByRole('alert', { description: 'doc-2025', exact: true }).evaluateAll(els => els.map(e => e.textContent)))));
  (await tier2.record('L325', async () => (await page.getByRole('alert', { description: 'File doc-2025.pdf was uploaded successfully', exact: true }).evaluateAll(els => els.map(e => e.textContent)))));

  // description regex match
  (await tier2.record('L330', async () => (await page.getByRole('alert', { description: /report-\d+/ }).evaluateAll(els => els.map(e => e.textContent)))));
  (await tier2.record('L333', async () => (await page.getByRole('alert', { description: /uploaded successfully$/ }).evaluateAll(els => els.map(e => e.textContent)))));
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
