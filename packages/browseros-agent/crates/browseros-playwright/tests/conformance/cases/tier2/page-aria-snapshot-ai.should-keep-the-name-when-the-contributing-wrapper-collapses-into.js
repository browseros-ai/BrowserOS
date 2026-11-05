// Upstream: Playwright v1.64.1 tests/page/page-aria-snapshot-ai.spec.ts:398
// Test: should keep the name when the contributing wrapper collapses into repeating text
// Lifted by tier2/convert.mjs. Rerun it instead of editing this file.
const tier2 = tier2Runtime(fixtures)
async function snapshotForAI(page, options) {
  return await page.ariaSnapshot({ ...options, mode: 'ai' });
}
return tier2.done(await harness.settle((async () => {
  // The outer span is marked as represented while it still wraps the icon. Once the nameless
  // image is dropped, the span collapses into a lone text repeating the button's name and removes
  // itself, so the button must keep the name derived from it.
  await page.goto(tier2.html(`
    <button>
      <span>
        <span>
          <svg focusable="false" tabindex="-1" aria-hidden="true" viewBox="0 0 448 512">
            <path d="M416 208H272V64c0-17.67-14.33-32-32-32h-32c-17.67 0-32 14.33-32 32v144H32c-17.67 0-32 14.33-32 32v32c0 17.67 14.33 32 32 32h144v144c0 17.67 14.33 32 32 32h32c17.67 0 32-14.33 32-32V304h144c17.67 0 32-14.33 32-32v-32c0-17.67-14.33-32-32-32z"/>
          </svg>
        </span>
        <span>Add New Item</span>
      </span>
    </button>
  `));

  const snapshot = await snapshotForAI(page);
  tier2.record('L417', () => (snapshot));
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
