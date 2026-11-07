// page.getByRef(ref) is Playwright's `locator('aria-ref=' + ref)`: a ref from
// `page.ariaSnapshot({ mode: 'ai' })` names that element for later calls, until a later snapshot
// renames it. Each engine reads its own snapshot, takes the ref of the "Two" button, reads and
// clicks it; a ref no snapshot gave matches nothing (count 0, and a read times out). Cases after
// `AR-048 should generate refs` (tests/page/page-aria-snapshot-ai.spec.ts at 26137face).
import assert from 'node:assert/strict'
import { test } from 'node:test'
import { compareMethod, skip } from './harness.mjs'

// BrowserOS neo's takeover bar shows in accessibility snapshots, on some tabs and not others; it
// is not the page's, so the page removes it on sight (as frame-top.html does).
const HTML = `<!doctype html>
<meta charset="utf-8">
<title>Refs</title>
<button id="one" type="button">One</button>
<button id="two" type="button">Two</button>
<button id="three" type="button">Three</button>
<script>
new MutationObserver(() => document.querySelector('browseros-neo-takeover')?.remove()).observe(
  document.documentElement, { childList: true, subtree: true })
window.__clicks = []
for (const button of document.querySelectorAll('button'))
  button.addEventListener('click', () => window.__clicks.push(button.id))
</script>`

test('getByRef resolves a ref from an ai snapshot', {
  skip,
  timeout: 120_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html: HTML,
    source: `
      const snapshot = await page.ariaSnapshot({ mode: 'ai' })
      const ref = /button "Two" \\[ref=(e\\d+)\\]/.exec(snapshot)?.[1]
      if (!ref) throw new Error('no ref for "Two": ' + snapshot)
      const two = page.getByRef(ref)
      const text = await two.textContent()
      await two.click()
      const unknown = page.getByRef('e999')
      const count = await unknown.count()
      let missing = 'resolved'
      try { await unknown.textContent({ timeout: 500 }) } catch (error) { missing = errorCategory(error) }
      return { text, count, missing, selector: two.toString() === "locator('aria-ref=" + ref + "')" }
    `,
    probe: 'window.__clicks',
  })
  assert.deepEqual(outcome.value, {
    text: 'Two',
    count: 0,
    missing: 'timeout',
    selector: true,
  })
})
