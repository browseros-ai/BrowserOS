// Page.waitForTimeout and Frame.waitForTimeout through Neo and Playwright: each resolves
// undefined after at least the delay (none for 0 or a negative delay), a child frame's wait ends
// with an Error when the frame is removed, and a missing, `null` or string delay is rejected by
// kind. Timings are compared only as "at least the delay".
//
// No pinned spec covers it alone (26137face specs only call it); the cases follow the client's
// `Frame.waitForTimeout` (sends `waitTimeout`) and the server's, which races page close and
// frame detach.
import assert from 'node:assert/strict'
import { test } from 'node:test'
import { compareMethod, skip } from './harness.mjs'

test('waitForTimeout waits its delay and ends with its frame', {
  skip,
  timeout: 120_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html: `<button id="remove" onclick="document.querySelector('iframe').remove()">Remove</button>
      <iframe name="child" srcdoc="<p>child</p>"></iframe>`,
    source: `
      const started = Date.now()
      const voids = [
        await page.waitForTimeout(150),
        await page.mainFrame().waitForTimeout(0),
        await page.waitForTimeout(-1),
      ].map((value) => typeof value)
      const waited = Date.now() - started >= 140
      const child = page.frame('child')
      await child.waitForTimeout(10)
      const ending = child.waitForTimeout(10000).then(() => 'resolved', errorCategory)
      await page.click('#remove')
      const detached = await ending
      const settled = []
      for (const delay of [undefined, null, '100']) {
        try { await page.waitForTimeout(delay); settled.push('resolved') } catch (error) { settled.push(errorCategory(error)) }
      }
      return { voids, waited, detached, settled }
    `,
    probe: "document.querySelectorAll('iframe').length",
  })
  assert.deepEqual(outcome.value, {
    voids: ['undefined', 'undefined', 'undefined'],
    waited: true,
    detached: 'Error',
    settled: ['Error', 'Error', 'Error'],
  })
})
