// page.addScriptTag and frame.addScriptTag through the Playwright reference and through Neo on
// fresh pages: a `<script>` from a URL or from content (classic or module) in the page's head,
// answered as an element handle once it loaded (or ran). No source, a URL that fails to load and
// a page whose Content Security Policy blocks the script are refused, and no script runs.
// Values by `describe` (values.mjs), errors by category and the words Playwright's tests look
// for; the page's state by `probe`. URLs are `data:` scripts (the fixture server answers every
// path with HTML) and an unreachable port.
//
// `path`: Playwright's client reads the file into `content`. Neo reads files only from the run's
// artifact directory (eval B1), so it refuses the option, naming it. Its case asserts each side.
// A `null` path is ignored, as Playwright's client (`if (copy.path)`) and validator ignore it; a
// `null` url, content or type is refused before any effect (addTag-options-review.test.mjs).
//
// Cases from tests/page/page-add-script-tag.spec.ts at 26137face: AT-001 "should throw an error if
// no options are provided", AT-002 "should work with a url", AT-003 "should work with a url and
// type=module", AT-005 "should work with a content and type=module", AT-006 "should throw an error
// if loading from url fail", AT-007 "should work with a path", AT-009 "should work with content",
// AT-010 "should throw when added with content to the CSP page", AT-011 "should throw when added
// with URL to the CSP page".
import assert from 'node:assert/strict'
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { test } from 'node:test'
import { compareMethod, skip } from './harness.mjs'
import { VALUES } from './values.mjs'

const html = '<title>tags</title><p id="p">text</p>'
const csp = `<meta http-equiv="Content-Security-Policy" content="script-src 'self'"><title>csp</title><p id="p">text</p>`
const probe =
  '({ injected: window.__injected ?? null, scripts: [...document.head.querySelectorAll("script")].map((s) => s.type || "classic") })'

test('addScriptTag adds a loaded or run script as Playwright does', {
  skip,
  timeout: 180_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html,
    source: `${VALUES}
      const tag = async (options) => (await page.addScriptTag(options)).evaluate((e) => [e.tagName, e.type])
      return {
        none: await outcome(() => page.addScriptTag(), ['Provide an object with a']),
        url: await outcome(() => tag({ url: 'data:text/javascript,window.__injected = 42' })),
        afterUrl: await page.evaluate(() => window.__injected),
        urlModule: await outcome(async () => {
          await page.addScriptTag({ url: 'data:text/javascript,window.__urlModule = 7', type: 'module' })
          return (await page.waitForFunction(() => window.__urlModule)).jsonValue()
        }),
        content: await outcome(async () => [await tag({ content: 'window.__content = 35' }), await page.evaluate(() => window.__content)]),
        contentModule: await outcome(async () => {
          await page.addScriptTag({ content: 'window.__contentModule = 8', type: 'module' })
          return (await page.waitForFunction(() => window.__contentModule)).jsonValue()
        }),
        fails: await outcome(() => page.addScriptTag({ url: 'http://127.0.0.1:1/missing.js' }), ['Failed to load script at']),
        frameForm: await outcome(async () => (await page.mainFrame().addScriptTag({ content: 'window.__frame = 1' })).evaluate((e) => e.text)),
        pathNull: await outcome(async () => [await tag({ path: null, content: 'window.__pathNull = 5' }), await page.evaluate(() => window.__pathNull)]),
      }
    `,
    probe,
  })
  assert.deepEqual(outcome.value.afterUrl, 42)
  assert.deepEqual(
    outcome.value.pathNull,
    { value: [['SCRIPT', 'text/javascript'], 5] },
    'a null path is no path',
  )
  assert.equal(outcome.value.fails.error, 'Error')
})

test('a page whose Content Security Policy blocks the script refuses the tag', {
  skip,
  timeout: 180_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html: csp,
    source: `${VALUES}
      return {
        content: await outcome(() => page.addScriptTag({ content: 'window.__injected = 35' })),
        url: await outcome(() => page.addScriptTag({ url: 'data:text/javascript,window.__injected = 36' })),
      }
    `,
    probe,
  })
  assert.deepEqual(outcome.value, {
    content: { error: 'Error', mentions: [] },
    url: { error: 'Error', mentions: [] },
  })
})

test('path: Playwright reads the file, Neo refuses the option', {
  skip,
  timeout: 180_000,
}, async (t) => {
  const dir = mkdtempSync(join(tmpdir(), 'neo-add-script-tag-'))
  try {
    const file = join(dir, 'injected.js')
    writeFileSync(file, 'window.__injected = 99')
    await compareMethod(t, {
      html,
      source: `
        const result = await page.addScriptTag({ path: ${JSON.stringify(file)} }).then(
          () => 'resolved',
          (error) => (error.message.includes('path') ? 'refused: path' : error.message),
        )
        await page.evaluate((recorded) => { window.__pathOutcome = recorded }, result)
        return null
      `,
      probe:
        '({ injected: window.__injected ?? null, outcome: window.__pathOutcome })',
      effects: ({ playwright, neo }) => {
        assert.deepEqual(
          playwright,
          { injected: 99, outcome: 'resolved' },
          'Playwright',
        )
        assert.deepEqual(
          neo,
          { injected: null, outcome: 'refused: path' },
          'Neo',
        )
      },
    })
  } finally {
    rmSync(dir, { recursive: true, force: true })
  }
})
