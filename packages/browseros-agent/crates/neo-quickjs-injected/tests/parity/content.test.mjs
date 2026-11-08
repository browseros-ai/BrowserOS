// Page.content and Frame.content through Neo and Playwright: the document's markup with its
// doctype, an open shadow root serialized only with `includeShadow`, the same markup through
// `page` and `page.mainFrame()`, a child frame's own document, and `null` options settled as
// Playwright's client settles them (`{ ...options }` reads a `null` object as `{}`; its validator
// refuses a `null` `includeShadow`). The fixture removes BrowserOS's document-idle takeover UI, so
// the markup is the fixture's alone.
//
// From Playwright's specs at the pin (tests/page/page-set-content.spec.ts at 26137face): 'PA-020
// should work' and 'PA-025 should include shadow roots' read markup back through content().
import assert from 'node:assert/strict'
import { test } from 'node:test'
import { compareMethod, skip } from './harness.mjs'

/** Keeps BrowserOS's takeover UI out of the document, as frame-top.html does. */
const ISOLATE = `<script>new MutationObserver(() => document.querySelector('browseros-neo-takeover')?.remove()).observe(document.documentElement, { childList: true, subtree: true })</script>`

const HTML = `<!doctype html><html><head><title>content</title></head><body>
<p id="text">Hello</p><div id="host"></div>
<iframe name="child" srcdoc="<p id='inner'>Inner</p>"></iframe>
${ISOLATE}<script>document.querySelector('#host').attachShadow({ mode: 'open' }).innerHTML = '<b>in' + 'side</b>'</script>
</body></html>`

test('content returns the document markup, its shadow roots only on request', {
  skip,
  timeout: 120_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html: HTML,
    source: `
      await page.frame('child').locator('#inner').waitFor()
      const plain = await page.content()
      const viaFrame = await page.mainFrame().content()
      const shadow = await page.content({ includeShadow: true })
      const child = await page.frame('child').content()
      const settled = []
      for (const options of [null, { includeShadow: null }, { includeShadow: false }, {}]) {
        try { settled.push(typeof await page.content(options)) } catch (error) { settled.push(errorCategory(error)) }
      }
      try { settled.push(typeof await page.mainFrame().content(null)) } catch (error) { settled.push(errorCategory(error)) }
      return { plain, same: plain === viaFrame, shadow, child, settled }
    `,
    probe: 'document.title',
  })
  const { plain, shadow, child } = outcome.value
  assert.ok(plain.startsWith('<!DOCTYPE html>'), plain)
  assert.ok(plain.includes('<p id="text">Hello</p>'), plain)
  assert.ok(
    !plain.includes('<b>inside</b>'),
    'no shadow root without includeShadow',
  )
  assert.ok(shadow.includes('<b>inside</b>'), shadow)
  assert.ok(child.includes('<p id="inner">Inner</p>'), child)
  assert.ok(!child.includes('Hello'), "the child frame's own document")
  assert.equal(outcome.value.same, true)
  assert.deepEqual(outcome.value.settled, [
    'string',
    'Error',
    'string',
    'string',
    'string',
  ])
})
