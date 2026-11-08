// Cold and prewarmed controls from the final integration review. A child announces that the
// binding exists before the parent asks it to call. The cold case invokes no child-frame API.
// Extends Playwright 26137face tests/page/page-expose-function.spec.ts:
// EF-010 "should work on frames" and EF-011 "should work on frames before navigation".
import assert from 'node:assert/strict'
import { test } from 'node:test'
import { compareMethod, skip } from './harness.mjs'

const html = `<title>binding origin</title><p id="ready">ready</p>
<script>
if (location.search === '?child') {
  parent.postMessage({ ready: true, kind: typeof known }, '*')
  addEventListener('message', event => {
    if (event.data === 'go') known(41).then(
      value => parent.postMessage({ value }, '*'),
      error => parent.postMessage({ error: error.name }, '*'))
  })
}
</script>`

for (const warm of [false, true]) {
  test(`tests that a ${warm ? 'prewarmed' : 'cold'} OOPIF binding reaches its callback`, {
    skip,
    timeout: 180_000,
  }, async (t) => {
    await compareMethod(t, {
      html,
      isolate: true,
      source: `let callbacks = 0
await page.context().exposeFunction('known', value => { callbacks++; return value + 1 })
await page.evaluate(url => {
  addEventListener('message', event => {
    if (event.data.ready) window.childReady = event.data
    else window.result = event.data
  })
  const child = document.createElement('iframe')
  child.id = 'child'; child.src = url; document.body.append(child)
}, page.url().replace('127.0.0.1', 'localhost') + '?child')
const ready = await page.waitForFunction(() => window.childReady, null, { timeout: 5000 })
await ready.dispose()
${warm ? "await page.frameLocator('#child').locator('#ready').waitFor({ state: 'attached', timeout: 5000 })" : ''}
await page.evaluate(() => document.querySelector('#child').contentWindow.postMessage('go', '*'))
try {
  const result = await page.waitForFunction(() => window.result, null, { timeout: 2000 })
  await result.dispose()
} catch (error) { if (error.name !== 'TimeoutError') throw error }
return { callbacks, ready: await page.evaluate(() => window.childReady), result: await page.evaluate(() => window.result ?? null) }`,
      probe: 'document.title',
      values({ playwright, neo }) {
        const expected = {
          callbacks: 1,
          ready: { ready: true, kind: 'function' },
          result: { value: 42 },
        }
        assert.deepEqual(playwright, expected)
        assert.deepEqual(neo, playwright)
      },
    })
  })
}
