// Locator.highlight shows Playwright's overlay over every match, in the document its selector
// reaches, and resolves to a disposable whose dispose() hides that highlight once: the main
// document gains the overlay, a frame locator's highlight goes to the child document only, a
// missing element resolves with nothing to show, and an invalid selector rejects.
//
// Where the two differ by design: Playwright keeps a highlight until it is hidden or the page
// closes; Neo hides every highlight of the run, and removes its overlays, when the run ends. So
// after the script Playwright's documents still hold their overlay and Neo's hold none, and the
// overlay log shows Neo's removals at the end.
//
// From Playwright's specs at the pin (tests/page/locator-highlight.spec.ts at 26137face): 'LO-114
// should highlight locator'. Its tooltip and box checks read the overlay's shadow root, which is
// open only under Playwright's own test runner; here it is closed in both engines.
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { test } from 'node:test'
import { compareMethod, skip } from './harness.mjs'

test('highlight places the overlay where Playwright does; Neo removes it at run end', {
  skip,
  timeout: 120_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html: readFileSync(
      new URL('../pages/highlight.html', import.meta.url),
      'utf8',
    ),
    source: `
      const stub = await page.locator('#target').highlight()
      const shape = { stub: typeof stub, dispose: typeof stub.dispose, json: JSON.stringify(stub) }
      const disposed = [await stub.dispose(), await stub.dispose()].map((value) => value === undefined)
      const voids = [
        await page.locator('#other').highlight({ style: { outlineColor: 'red', zIndex: 5 } }),
        await page.locator('.dup').highlight({ style: 'outline: 2px solid red' }),
        await page.locator('#missing').highlight(),
        await page.frameLocator('#child').locator('#inner').highlight(),
      ].map((value) => typeof value)
      let invalid
      try { await page.locator('[').highlight(); invalid = 'resolved' } catch (error) { invalid = errorCategory(error) }
      return { shape, disposed, voids, invalid }
    `,
    probe: 'window.__facts()',
    effects: ({ playwright, neo }) => {
      assert.deepEqual(playwright, {
        log: ['+main', '+child'],
        main: 1,
        child: 1,
      })
      assert.deepEqual(neo, {
        log: ['+main', '+child', '-main', '-child'],
        main: 0,
        child: 0,
      })
    },
  })
  assert.deepEqual(outcome.value, {
    shape: { stub: 'object', dispose: 'function', json: '{}' },
    disposed: [true, true],
    voids: ['object', 'object', 'object', 'object'],
    invalid: 'Error',
  })
})
