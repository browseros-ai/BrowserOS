// Upstream: Playwright v1.64.1 tests/page/selectors-text.spec.ts:20
// Test: should work @smoke
// Lifted by tier2/convert.mjs. Rerun it instead of editing this file.
// @needs R10c: $$eval, $eval, locator.evaluate (R10c) not built yet
const tier2 = tier2Runtime(fixtures)
return tier2.done(await harness.settle((async () => {
  await page.goto(tier2.html(`<div>yo</div><div>ya</div><div>\nye  </div>`));
  (await tier2.record('L22', async () => (await page.$eval(`text=ya`, e => e.outerHTML))));
  (await tier2.record('L23', async () => (await page.$eval(`text="ya"`, e => e.outerHTML))));
  (await tier2.record('L24', async () => (await page.$eval(`text=/^[ay]+$/`, e => e.outerHTML))));
  (await tier2.record('L25', async () => (await page.$eval(`text=/Ya/i`, e => e.outerHTML))));
  (await tier2.record('L26', async () => (await page.$eval(`text=ye`, e => e.outerHTML))));
  (await tier2.record('L27', async () => (await page.getByText('ye').evaluate(e => e.outerHTML))));

  await page.goto(tier2.html(`<div> ye </div><div>ye</div>`));
  (await tier2.record('L30', async () => (await page.$eval(`text="ye"`, e => e.outerHTML))));
  (await tier2.record('L31', async () => (await page.getByText('ye', { exact: true }).first().evaluate(e => e.outerHTML))));

  await page.goto(tier2.html(`<div>yo</div><div>"ya</div><div> hello world! </div>`));
  (await tier2.record('L34', async () => (await page.$eval(`text="\\"ya"`, e => e.outerHTML))));
  (await tier2.record('L35', async () => (await page.$eval(`text=/hello/`, e => e.outerHTML))));
  (await tier2.record('L36', async () => (await page.$eval(`text=/^\\s*heLLo/i`, e => e.outerHTML))));

  await page.goto(tier2.html(`<div>yo<div>ya</div>hey<div>hey</div></div>`));
  (await tier2.record('L39', async () => (await page.$eval(`text=hey`, e => e.outerHTML))));
  (await tier2.record('L40', async () => (await page.$eval(`text=yo>>text="ya"`, e => e.outerHTML))));
  (await tier2.record('L41', async () => (await page.$eval(`text=yo>> text="ya"`, e => e.outerHTML))));
  (await tier2.record('L42', async () => (await page.$eval(`text=yo >>text='ya'`, e => e.outerHTML))));
  (await tier2.record('L43', async () => (await page.$eval(`text=yo >> text='ya'`, e => e.outerHTML))));
  (await tier2.record('L44', async () => (await page.$eval(`'yo'>>"ya"`, e => e.outerHTML))));
  (await tier2.record('L45', async () => (await page.$eval(`"yo" >> 'ya'`, e => e.outerHTML))));

  await page.goto(tier2.html(`<div>yo<span id="s1"></span></div><div>yo<span id="s2"></span><span id="s3"></span></div>`));
  (await tier2.record('L48', async () => (await page.$$eval(`text=yo`, es => es.map(e => e.outerHTML).join('\n')))));

  await page.goto(tier2.html(`<div>'</div><div>"</div><div>\\</div><div>x</div>`));
  (await tier2.record('L51', async () => (await page.$eval(`text='\\''`, e => e.outerHTML))));
  (await tier2.record('L52', async () => (await page.$eval(`text='"'`, e => e.outerHTML))));
  (await tier2.record('L53', async () => (await page.$eval(`text="\\""`, e => e.outerHTML))));
  (await tier2.record('L54', async () => (await page.$eval(`text="'"`, e => e.outerHTML))));
  (await tier2.record('L55', async () => (await page.$eval(`text="\\x"`, e => e.outerHTML))));
  (await tier2.record('L56', async () => (await page.$eval(`text='\\x'`, e => e.outerHTML))));
  (await tier2.record('L57', async () => (await page.$eval(`text='\\\\'`, e => e.outerHTML))));
  (await tier2.record('L58', async () => (await page.$eval(`text="\\\\"`, e => e.outerHTML))));
  (await tier2.record('L59', async () => (await page.$eval(`text="`, e => e.outerHTML))));
  (await tier2.record('L60', async () => (await page.$eval(`text='`, e => e.outerHTML))));
  (await tier2.record('L61', async () => (await page.$eval(`"x"`, e => e.outerHTML))));
  (await tier2.record('L62', async () => (await page.$eval(`'x'`, e => e.outerHTML))));
  let error = await page.$(`"`).catch(e => e);
  tier2.record('L64', () => (error));
  error = await page.$(`'`).catch(e => e);
  tier2.record('L66', () => (error));

  await page.goto(tier2.html(`<div> ' </div><div> " </div>`));
  (await tier2.record('L69', async () => (await page.$eval(`text="`, e => e.outerHTML))));
  (await tier2.record('L70', async () => (await page.$eval(`text='`, e => e.outerHTML))));

  await page.goto(tier2.html(`<div>Hi''&gt;&gt;foo=bar</div>`));
  (await tier2.record('L73', async () => (await page.$eval(`text="Hi''>>foo=bar"`, e => e.outerHTML))));
  await page.goto(tier2.html(`<div>Hi'"&gt;&gt;foo=bar</div>`));
  (await tier2.record('L75', async () => (await page.$eval(`text="Hi'\\">>foo=bar"`, e => e.outerHTML))));

  await page.goto(tier2.html(`<div>Hi&gt;&gt;<span></span></div>`));
  (await tier2.record('L78', async () => (await page.$eval(`text="Hi>>">>span`, e => e.outerHTML))));
  (await tier2.record('L79', async () => (await page.$eval(`text=/Hi\\>\\>/ >> span`, e => e.outerHTML))));

  await page.goto(tier2.html(`<div>a<br>b</div><div>a</div>`));
  (await tier2.record('L82', async () => (await page.$eval(`text=a`, e => e.outerHTML))));
  (await tier2.record('L83', async () => (await page.$eval(`text=b`, e => e.outerHTML))));
  (await tier2.record('L84', async () => (await page.$eval(`text=ab`, e => e.outerHTML))));
  (await tier2.record('L85', async () => (await page.$(`text=abc`))));
  (await tier2.record('L86', async () => (await page.$$eval(`text=a`, els => els.length))));
  (await tier2.record('L87', async () => (await page.$$eval(`text=b`, els => els.length))));
  (await tier2.record('L88', async () => (await page.$$eval(`text=ab`, els => els.length))));
  (await tier2.record('L89', async () => (await page.$$eval(`text=abc`, els => els.length))));

  await page.goto(tier2.html(`<div></div><span></span>`));
  await page.$eval('div', div => {
    div.appendChild(document.createTextNode('hello'));
    div.appendChild(document.createTextNode('world'));
  });
  await page.$eval('span', span => {
    span.appendChild(document.createTextNode('hello'));
    span.appendChild(document.createTextNode('world'));
  });
  (await tier2.record('L100', async () => (await page.$eval(`text=lowo`, e => e.outerHTML))));
  (await tier2.record('L101', async () => (await page.$$eval(`text=lowo`, els => els.map(e => e.outerHTML).join('')))));

  await page.goto(tier2.html(`<span>Sign&nbsp;in</span><span>Hello\n \nworld</span>`));
  (await tier2.record('L104', async () => (await page.$eval(`text=Sign in`, e => e.outerHTML))));
  (await tier2.record('L105', async () => ((await page.$$(`text=Sign \tin`)).length)));
  (await tier2.record('L106', async () => ((await page.$$(`text="Sign in"`)).length)));
  (await tier2.record('L107', async () => (await page.$eval(`text=lo wo`, e => e.outerHTML))));
  (await tier2.record('L108', async () => (await page.$eval(`text="Hello world"`, e => e.outerHTML))));
  (await tier2.record('L109', async () => (await page.$(`text="lo wo"`))));
  (await tier2.record('L110', async () => ((await page.$$(`text=lo \nwo`)).length)));
  (await tier2.record('L111', async () => ((await page.$$(`text="lo \nwo"`)).length)));

  await page.goto(tier2.html(`<div>let's<span>hello</span></div>`));
  (await tier2.record('L114', async () => (await page.$eval(`text=/let's/i >> span`, e => e.outerHTML))));
  (await tier2.record('L115', async () => (await page.$eval(`text=/let\\'s/i >> span`, e => e.outerHTML))));
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
