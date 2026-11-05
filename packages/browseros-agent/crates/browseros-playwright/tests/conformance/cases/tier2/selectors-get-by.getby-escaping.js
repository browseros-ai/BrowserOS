// Upstream: Playwright v1.64.1 tests/page/selectors-get-by.spec.ts:187
// Test: getBy escaping
// Lifted by tier2/convert.mjs. Rerun it instead of editing this file.
// @needs R10c: $eval (R10c) not built yet
const tier2 = tier2Runtime(fixtures)
return tier2.done(await harness.settle((async () => {
  await page.goto(tier2.html(`<label id=label for=control>Hello my
wo"rld</label><input id=control />`));
  await page.$eval('input', input => {
    input.setAttribute('placeholder', 'hello my\nwo"rld');
    input.setAttribute('title', 'hello my\nwo"rld');
    input.setAttribute('alt', 'hello my\nwo"rld');
  });
  await tier2.record('L195', () => expect(page.getByText('hello my\nwo"rld')).toHaveAttribute('id', 'label'));
  await tier2.record('L196', () => expect(page.getByText('hello       my     wo"rld')).toHaveAttribute('id', 'label'));
  await tier2.record('L197', () => expect(page.getByLabel('hello my\nwo"rld')).toHaveAttribute('id', 'control'));
  await tier2.record('L198', () => expect(page.getByPlaceholder('hello my\nwo"rld')).toHaveAttribute('id', 'control'));
  await tier2.record('L199', () => expect(page.getByAltText('hello my\nwo"rld')).toHaveAttribute('id', 'control'));
  await tier2.record('L200', () => expect(page.getByTitle('hello my\nwo"rld')).toHaveAttribute('id', 'control'));

  await page.goto(tier2.html(`<label id=label for=control>Hello my
world</label><input id=control />`));
  await page.$eval('input', input => {
    input.setAttribute('placeholder', 'hello my\nworld');
    input.setAttribute('title', 'hello my\nworld');
    input.setAttribute('alt', 'hello my\nworld');
  });
  await tier2.record('L209', () => expect(page.getByText('hello my\nworld')).toHaveAttribute('id', 'label'));
  await tier2.record('L210', () => expect(page.getByText('hello        my    world')).toHaveAttribute('id', 'label'));
  await tier2.record('L211', () => expect(page.getByLabel('hello my\nworld')).toHaveAttribute('id', 'control'));
  await tier2.record('L212', () => expect(page.getByPlaceholder('hello my\nworld')).toHaveAttribute('id', 'control'));
  await tier2.record('L213', () => expect(page.getByAltText('hello my\nworld')).toHaveAttribute('id', 'control'));
  await tier2.record('L214', () => expect(page.getByTitle('hello my\nworld')).toHaveAttribute('id', 'control'));

  await page.goto(tier2.html(`<div id=target title="my title">Text here</div>`));
  await tier2.record('L217', () => expect(page.getByTitle('my title', { exact: true })).toHaveCount(1, { timeout: 500 }));
  await tier2.record('L218', () => expect(page.getByTitle('my t\itle', { exact: true })).toHaveCount(1, { timeout: 500 }));
  await tier2.record('L219', () => expect(page.getByTitle('my t\\itle', { exact: true })).toHaveCount(0, { timeout: 500 }));
  await tier2.record('L220', () => expect(page.getByTitle('my t\\\itle', { exact: true })).toHaveCount(0, { timeout: 500 }));
  await tier2.record('L221', () => expect(page.getByTitle('my t\\\\itle', { exact: true })).toHaveCount(0, { timeout: 500 }));

  await page.goto(tier2.html(`<label for=target>foo &gt;&gt; bar</label><input id=target>`));
  await page.$eval('input', input => {
    input.setAttribute('placeholder', 'foo >> bar');
    input.setAttribute('title', 'foo >> bar');
    input.setAttribute('alt', 'foo >> bar');
  });
  (await tier2.record('L229', async () => (await page.getByText('foo >> bar').textContent())));
  await tier2.record('L230', () => expect(page.locator('label')).toHaveText('foo >> bar'));
  await tier2.record('L231', () => expect(page.getByText('foo >> bar')).toHaveText('foo >> bar'));
  (await tier2.record('L232', async () => (await page.getByText(/foo >> bar/).textContent())));
  await tier2.record('L233', () => expect(page.getByLabel('foo >> bar')).toHaveAttribute('id', 'target'));
  await tier2.record('L234', () => expect(page.getByLabel(/foo >> bar/)).toHaveAttribute('id', 'target'));
  await tier2.record('L235', () => expect(page.getByPlaceholder('foo >> bar')).toHaveAttribute('id', 'target'));
  await tier2.record('L236', () => expect(page.getByAltText('foo >> bar')).toHaveAttribute('id', 'target'));
  await tier2.record('L237', () => expect(page.getByTitle('foo >> bar')).toHaveAttribute('id', 'target'));
  await tier2.record('L238', () => expect(page.getByPlaceholder(/foo >> bar/)).toHaveAttribute('id', 'target'));
  await tier2.record('L239', () => expect(page.getByAltText(/foo >> bar/)).toHaveAttribute('id', 'target'));
  await tier2.record('L240', () => expect(page.getByTitle(/foo >> bar/)).toHaveAttribute('id', 'target'));
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
