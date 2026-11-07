// browser.newContext through Neo and through Playwright on one BrowserOS neo, on fresh tabs of
// tests/pages/emulation.html: a new context shares no cookies or storage with the script's own
// (whose page wrote both), its pages emulate its options (viewport, color scheme, reduced motion,
// locale, time zone, user agent), invalid option combinations are refused, and the context and its
// page close together.
//
// Playwright cases (pin 26137face): `tests/library/browser.spec.ts` "BR-006 should fire context
// event on newContext"; `tests/library/browsercontext-viewport.spec.ts` "VP-002 should set the
// proper viewport size".
//
// Neo only: an option Neo does not serve yet (`proxy`) is refused by name, and a context the
// script leaves open is disposed, with its page, when the run ends (Playwright keeps it until it
// is closed, so neither has a Playwright side).

import assert from 'node:assert/strict'
import { test } from 'node:test'
import { skip } from './harness.mjs'
import { fixturePage, outcomeValue, withPlatform } from './platform.mjs'

const pages = { '/emulation.html': fixturePage('emulation.html') }

const cases = [
  {
    name: 'isolated from the script context',
    source: `const browser = context.browser()
const own = await settle((s) => s.cookie !== '')
const created = await browser.newContext()
const other = await created.newPage()
await other.goto(page.url().split('?')[0])
const fresh = await settle(() => true, other)
const result = {
  own: [own.cookie, own.localStorage],
  created: [fresh.cookie, fresh.localStorage],
  pages: created.pages().length,
  first: created.pages()[0] === other,
  ownListsIt: context.pages().includes(other),
  browser: created.browser() === browser,
}
await created.close()
result.closed = [other.isClosed(), created.pages().length, browser.contexts().includes(created)]
return result`,
    expected: {
      own: ['neo-parity=1', '1'],
      created: ['', null],
      pages: 1,
      first: true,
      ownListsIt: false,
      browser: true,
      closed: [true, 0, false],
    },
  },
  {
    name: 'its pages emulate its options',
    source: `const created = await context.browser().newContext({
  viewport: { width: 420, height: 380 }, colorScheme: 'dark', reducedMotion: 'reduce',
  locale: 'de-DE', timezoneId: 'Asia/Tokyo', userAgent: 'NeoParity/1.0',
})
const other = await created.newPage()
await other.goto(page.url())
const s = await settle((s) => s.innerWidth === 420, other)
await created.close()
return { size: [s.innerWidth, s.innerHeight], dark: s.media['(prefers-color-scheme: dark)'],
  reduce: s.media['(prefers-reduced-motion: reduce)'], language: s.language, timeZone: s.timeZone,
  userAgent: s.userAgent, viewport: other.viewportSize() }`,
    expected: {
      size: [420, 380],
      dark: true,
      reduce: true,
      language: 'de-DE',
      timeZone: 'Asia/Tokyo',
      userAgent: 'NeoParity/1.0',
      viewport: { width: 420, height: 380 },
    },
  },
  {
    name: 'invalid option combinations are refused',
    source: `const browser = context.browser()
const refused = []
for (const options of [{ viewport: null, isMobile: true }, { viewport: null, deviceScaleFactor: 2 }, { viewport: { width: 'wide', height: 1 } }])
  refused.push(await browser.newContext(options).then(() => 'created', errorCategory))
return { refused, contexts: browser.contexts().length }`,
    expected: { refused: ['Error', 'Error', 'Error'], contexts: 1 },
  },
]

test('browser.newContext: Neo matches Playwright', { skip }, async (t) => {
  await withPlatform(t, { pages }, async ({ playwright, neo, cdp }) => {
    for (const { name, source, expected } of cases) {
      const reference = outcomeValue(
        await playwright('/emulation.html?storage=set', source),
      )
      assert.deepEqual(reference, expected, `${name}: Playwright`)
      assert.deepEqual(
        outcomeValue(await neo('/emulation.html?storage=set', source)),
        reference,
        `${name}: Neo`,
      )
    }
    // Neo only: an unserved option is refused by name; a context left open ends with the run.
    const neoOnly = await neo(
      '/emulation.html',
      `const browser = context.browser()
const proxy = await browser.newContext({ proxy: { server: 'http://127.0.0.1:9' } }).then(() => 'created', (e) => [errorCategory(e), /"proxy"/.test(e.message)])
const left = await browser.newContext()
const leftPage = await left.newPage()
return { proxy, open: browser.contexts().length }`,
    )
    assert.deepEqual(outcomeValue(neoOnly), { proxy: ['Error', true], open: 2 })
    const { browserContextIds } = await cdp.send('Target.getBrowserContexts')
    assert.deepEqual(
      browserContextIds,
      [],
      'the run disposed the context it left open',
    )
    const created = neoOnly.outcome.pages.filter((p) => p.created)
    assert.deepEqual(
      created.map((p) => p.closed),
      [true],
      'the outcome lists the created page as closed by the run',
    )
  })
})
