// page.addInitScript and browserContext.addInitScript through the Playwright reference and through
// Neo: a script (a function with its argument, source text, or `{ content }`) runs before the
// page's own scripts in every new document, in the page's main world, in order, and its
// Disposable removes it again (a second dispose is done). A frame the page adds later runs them
// too. A context script reaches the pages
// that already exist and those the context opens later. On frame-top.html (frames.mjs) the
// script runs in every frame of the new document, same-process children and cross-site
// out-of-process iframes alike, before each frame's own script. Values by `describe`
// (values.mjs), errors by category and the words Playwright's tests look for.
//
// `path`: Playwright's client reads the file into the script. Neo reads files only from the run's
// artifact directory, which a run has not yet (eval B1), so it refuses the option, naming it
// (`Deviation:` in init-script.ts); that case asserts each side. Neo removes every script a run
// added when the run ends (`Deviation:` in init_scripts.rs); `neo-runtime/tests/init_scripts_e2e.rs`
// and `init_scripts_lifecycle.rs` show a later document never runs one.
//
// Cases from tests/page/page-add-init-script.spec.ts at 26137face: IS-001 "should evaluate before
// anything else on the page", IS-003 "should work with content", IS-004 "should throw without
// path and content", IS-006 "should support multiple scripts", IS-009 "should remove init script
// after dispose", IS-010 "should remove one of multiple init scripts after dispose"; and
// tests/library/browsercontext-add-init-script.spec.ts: IS-024 "should work with browser context
// scripts", IS-028 "should work with browser context scripts for already created pages", IS-029
// "should remove context init script after dispose", IS-030 "should remove context init script
// and keep working in new pages".
import assert from 'node:assert/strict'
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { test } from 'node:test'
import { compareFrameCases, FRAME_TREE } from './frames.mjs'
import { compareMethod, skip } from './harness.mjs'
import { VALUES } from './values.mjs'

const html =
  '<title>init</title><script>window.result = window.injected; window.order = (window.order || []).concat("page")</script>'

test('page.addInitScript runs before the page in every new document', {
  skip,
  timeout: 180_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html,
    source: `${VALUES}
      const results = {}
      const first = await page.addInitScript(function () {
        window.injected = 123
        window.order = (window.order || []).concat('init1')
      })
      await page.addInitScript(function () {
        window.script2 = 2
        window.order = (window.order || []).concat('init2')
      })
      await page.addInitScript({ content: 'window.content = "c"' })
      await page.addInitScript((arg) => { window.withArg = arg }, { a: [1, 'b'] })
      results.noContent = await outcome(() => page.addInitScript({ foo: 'bar' }), ['Either path or content property must be present'])
      results.stringWithArg = await outcome(() => page.addInitScript('1', 2), ['Cannot evaluate a string with arguments'])
      results.current = await page.evaluate(() => window.injected ?? null)
      await page.goto(page.url())
      results.after = await page.evaluate(() => ({ result: window.result, order: window.order, script2: window.script2, content: window.content, withArg: window.withArg }))
      results.child = await page.evaluate(() => new Promise((resolve) => {
        const frame = document.createElement('iframe')
        frame.src = location.pathname + '?child'
        frame.onload = () => resolve({ result: frame.contentWindow.result, order: frame.contentWindow.order })
        document.body.append(frame)
      }))
      await first.dispose()
      results.disposeAgain = await outcome(() => first.dispose())
      await page.goto(page.url())
      results.afterDispose = await page.evaluate(() => ({ result: window.result ?? null, order: window.order, script2: window.script2 }))
      return results
    `,
    probe: '({ result: window.result ?? null, order: window.order })',
  })
  assert.deepEqual(outcome.value.after, {
    result: 123,
    order: ['init1', 'init2', 'page'],
    script2: 2,
    content: 'c',
    withArg: { a: [1, 'b'] },
  })
  assert.deepEqual(
    outcome.value.child,
    { result: 123, order: ['init1', 'init2', 'page'] },
    'a frame the page adds later runs them first too',
  )
  assert.deepEqual(
    outcome.value.current,
    null,
    'the current document never runs it',
  )
  assert.deepEqual(outcome.value.afterDispose, {
    result: null,
    order: ['init2', 'page'],
    script2: 2,
  })
})

test('browserContext.addInitScript reaches existing and new pages', {
  skip,
  timeout: 180_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html,
    source: `${VALUES}
      const browserContext = page.context()
      const script = await browserContext.addInitScript(() => { window.injected = 'context' })
      await page.goto(page.url())
      const existing = await page.evaluate(() => window.result ?? null)
      const second = await browserContext.newPage()
      await second.goto(page.url())
      const opened = await second.evaluate(() => window.result ?? null)
      await second.close()
      await script.dispose()
      await page.goto(page.url())
      const disposed = await page.evaluate(() => window.result ?? null)
      return { existing, opened, disposed }
    `,
    probe: '({ result: window.result ?? null })',
  })
  assert.deepEqual(outcome.value, {
    existing: 'context',
    opened: 'context',
    disposed: null,
  })
})

test('path: Playwright reads the file, Neo refuses the option', {
  skip,
  timeout: 180_000,
}, async (t) => {
  const dir = mkdtempSync(join(tmpdir(), 'neo-add-init-script-'))
  try {
    const file = join(dir, 'init.js')
    writeFileSync(file, 'window.injected = 99')
    await compareMethod(t, {
      html,
      source: `
        const result = await page.addInitScript({ path: ${JSON.stringify(file)} }).then(
          () => 'resolved',
          (error) => (error.message.includes('path') ? 'refused: path' : error.message),
        )
        await page.goto(page.url())
        await page.evaluate((recorded) => { window.__pathOutcome = recorded }, result)
        return null
      `,
      probe:
        '({ result: window.result ?? null, outcome: window.__pathOutcome })',
      effects: ({ playwright, neo }) => {
        assert.deepEqual(
          playwright,
          { result: 99, outcome: 'resolved' },
          'Playwright',
        )
        assert.deepEqual(neo, { result: null, outcome: 'refused: path' }, 'Neo')
      },
    })
  } finally {
    rmSync(dir, { recursive: true, force: true })
  }
})

const frameCases = [
  {
    name: 'an init script runs first in every frame of the new document, OOPIFs included',
    source: `await page.addInitScript(() => {
      window.__init = (window.__init || 0) + 1
      window.__initBefore = window.__who === undefined
    })
    await page.goto(page.url())
    return null`,
    probe:
      '({ init: window.__init ?? null, before: window.__initBefore ?? null })',
    expected: null,
    effect: (effect) => {
      for (const who of Object.keys(FRAME_TREE))
        assert.deepEqual(
          effect[who],
          { init: 1, before: true },
          `the init script ran first, once, in ${who}`,
        )
    },
  },
]

test('init scripts reach every frame through Neo as through Playwright', {
  skip,
  timeout: 300_000,
}, async (t) => {
  await compareFrameCases(t, frameCases)
})
