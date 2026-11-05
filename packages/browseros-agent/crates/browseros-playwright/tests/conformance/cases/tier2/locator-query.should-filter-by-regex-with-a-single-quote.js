// Upstream: Playwright v1.64.1 tests/page/locator-query.spec.ts:88
// Test: should filter by regex with a single quote
// Lifted by tier2/convert.mjs. Rerun it instead of editing this file.
const tier2 = tier2Runtime(fixtures)
return tier2.done(await harness.settle((async () => {
  await page.goto(tier2.html(`<button>let's let's<span>hello</span></button>`));
  await tier2.record('L90', () => expect(page.locator('button', { hasText: /let's/i }).locator('span')).toHaveText('hello'));
  await tier2.record('L91', () => expect(page.getByRole('button', { name: /let's/i }).locator('span')).toHaveText('hello'));
  await tier2.record('L92', () => expect(page.locator('button', { hasText: /let\'s/i }).locator('span')).toHaveText('hello'));
  await tier2.record('L93', () => expect(page.getByRole('button', { name: /let\'s/i }).locator('span')).toHaveText('hello'));
  await tier2.record('L94', () => expect(page.locator('button', { hasText: /'s/i }).locator('span')).toHaveText('hello'));
  await tier2.record('L95', () => expect(page.getByRole('button', { name: /'s/i }).locator('span')).toHaveText('hello'));
  await tier2.record('L96', () => expect(page.locator('button', { hasText: /\'s/i }).locator('span')).toHaveText('hello'));
  await tier2.record('L97', () => expect(page.getByRole('button', { name: /\'s/i }).locator('span')).toHaveText('hello'));
  await tier2.record('L98', () => expect(page.locator('button', { hasText: /let['abc]s/i }).locator('span')).toHaveText('hello'));
  await tier2.record('L99', () => expect(page.getByRole('button', { name: /let['abc]s/i }).locator('span')).toHaveText('hello'));
  await tier2.record('L100', () => expect(page.locator('button', { hasText: /let\\'s/i })).not.toBeVisible());
  await tier2.record('L101', () => expect(page.getByRole('button', { name: /let\\'s/i })).not.toBeVisible());
  await tier2.record('L102', () => expect(page.locator('button', { hasText: /let's let\'s/i }).locator('span')).toHaveText('hello'));
  await tier2.record('L103', () => expect(page.getByRole('button', { name: /let's let\'s/i }).locator('span')).toHaveText('hello'));
  await tier2.record('L104', () => expect(page.locator('button', { hasText: /let\'s let's/i }).locator('span')).toHaveText('hello'));
  await tier2.record('L105', () => expect(page.getByRole('button', { name: /let\'s let's/i }).locator('span')).toHaveText('hello'));

  await page.goto(tier2.html(`<button>let\\'s let\\'s<span>hello</span></button>`));
  await tier2.record('L108', () => expect(page.locator('button', { hasText: /let\'s/i })).not.toBeVisible());
  await tier2.record('L109', () => expect(page.getByRole('button', { name: /let\'s/i })).not.toBeVisible());
  await tier2.record('L110', () => expect(page.locator('button', { hasText: /let\\'s/i }).locator('span')).toHaveText('hello'));
  await tier2.record('L111', () => expect(page.getByRole('button', { name: /let\\'s/i }).locator('span')).toHaveText('hello'));
  await tier2.record('L112', () => expect(page.locator('button', { hasText: /let\\\'s/i }).locator('span')).toHaveText('hello'));
  await tier2.record('L113', () => expect(page.getByRole('button', { name: /let\\\'s/i }).locator('span')).toHaveText('hello'));
  await tier2.record('L114', () => expect(page.locator('button', { hasText: /let\\'s let\\\'s/i }).locator('span')).toHaveText('hello'));
  await tier2.record('L115', () => expect(page.getByRole('button', { name: /let\\'s let\\\'s/i }).locator('span')).toHaveText('hello'));
  await tier2.record('L116', () => expect(page.locator('button', { hasText: /let\\\'s let\\'s/i }).locator('span')).toHaveText('hello'));
  await tier2.record('L117', () => expect(page.getByRole('button', { name: /let\\\'s let\\'s/i }).locator('span')).toHaveText('hello'));

  await page.goto(tier2.html(`<button>let's hello</button>`));
  await tier2.record('L120', () => expect(page.locator('button', { hasText: /let's/iu })).toHaveText(`let's hello`));
  await tier2.record('L121', () => expect(page.getByRole('button', { name: /let's/iu })).toHaveText(`let's hello`));
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
