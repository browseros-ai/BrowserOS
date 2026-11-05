// Upstream: Playwright v1.64.1 tests/page/page-click-scroll.spec.ts:160
// Test: should scroll instantly on retry when scroll-behavior is smooth
// Lifted by tier2/convert.mjs. Rerun it instead of editing this file.
const tier2 = tier2Runtime(fixtures)
return tier2.done(await harness.settle((async () => {
  // The fixed header covers everything but the bottom strip of the viewport.
  // The first attempt centers the button under the header, so the click is
  // retried with a different scroll alignment using the native scrollIntoView().
  await page.goto(tier2.html(`
    <style>
      html { scroll-behavior: smooth; }
      body { margin: 0; }
      .spacer { height: 2000px; }
      #header { position: fixed; top: 0; left: 0; right: 0; bottom: 50px; background: rgba(0, 0, 0, 0.1); }
      button { height: 30px; }
    </style>
    <div id="header"></div>
    <div class="spacer"></div>
    <button onclick="window.clicked = true">Target</button>
    <div class="spacer"></div>
    <script>
      window.scrolls = [];
      addEventListener('scroll', () => window.scrolls.push(window.scrollY));
    </script>
  `));
  await page.click('button');
  (await tier2.record('L183', async () => (await page.evaluate('window.clicked'))));
  // Every scroll performed by Playwright should be a single instant jump,
  // and never a smooth animation that fires a scroll event per frame.
  const scrolls = await page.evaluate(() => window['scrolls']);
  tier2.record('L187', () => (scrolls.length));
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
