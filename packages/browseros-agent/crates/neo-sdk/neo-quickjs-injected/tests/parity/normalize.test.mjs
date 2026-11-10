// Locator.normalize through Neo and Playwright: a locator of Playwright's simple selector for
// the first match (not strict), behind the selector of each iframe on the way from the main
// frame, compared through toString() (Playwright's locator description) and through what the
// normalized locator reads. An ai snapshot's ref normalizes to the element it names; a locator
// with no match, a missing ref, and a missing frame are Errors.
//
// From Playwright's specs at the pin (tests/page/page-aria-snapshot-ai.spec.ts at 26137face):
// 'AR-052 should stitch all frame snapshots' (its normalize block) and 'AR-053' (a missing ref
// is No element matching).
import assert from 'node:assert/strict'
import { test } from 'node:test'
import { compareMethod, skip } from './harness.mjs'

const HTML = `<button>Top</button><button>Other</button>
<iframe name="outer" srcdoc="<p>outer</p><iframe name='inner' srcdoc='<button>Deep</button>'></iframe>"></iframe>`

test('normalize answers the simple selector of the first match, frames included', {
  skip,
  timeout: 120_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html: HTML,
    source: `
      const deepLocator = page.frameLocator('iframe[name=outer]').frameLocator('iframe[name=inner]').getByRole('button')
      await deepLocator.waitFor()
      const top = await page.getByRole('button', { name: 'Top' }).normalize()
      const first = await page.locator('button').normalize()
      const deep = await deepLocator.normalize()
      const viaFrame = await page.frame('inner').locator('button').normalize()
      const ai = await page.ariaSnapshot({ mode: 'ai' })
      const ref = ai.split('\\n').find((line) => line.includes('button "Other"'))?.match(/\\[ref=(e\\d+)\\]/)?.[1]
      const byRef = await page.getByRef(ref).normalize()
      const failures = {}
      for (const [name, locator] of [
        ['missing', page.locator('#none')],
        ['missingRef', page.getByRef('e1000')],
        ['missingFrame', page.frameLocator('#nope').locator('button')],
      ]) {
        failures[name] = await locator.normalize().then(() => 'resolved', errorCategory)
      }
      return {
        strings: [top, first, deep, viaFrame, byRef].map(String),
        texts: [await top.textContent(), await first.textContent(), await deep.textContent(), await byRef.textContent()],
        failures,
      }
    `,
    probe: 'document.title',
  })
  assert.deepEqual(outcome.value.texts, ['Top', 'Top', 'Deep', 'Other'])
  assert.deepEqual(outcome.value.failures, {
    missing: 'Error',
    missingRef: 'Error',
    missingFrame: 'Error',
  })
  const [, , deep, viaFrame] = outcome.value.strings
  assert.match(deep, /contentFrame\(\).*contentFrame\(\)/, 'two frame hops')
  assert.equal(viaFrame, deep, 'a Frame receiver resolves the same path')
})
