import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { test } from 'node:test'
import { compareMethod, skip } from './harness.mjs'

test('a drag timing out in the source handler never reports the whole drag done', {
  skip,
  timeout: 120_000,
}, async (t) => {
  const fixture = readFileSync(
    new URL('../pages/drag.html', import.meta.url),
    'utf8',
  )
  const outcome = await compareMethod(t, {
    html:
      fixture +
      `<script>
      window.__pressed = 0
      document.querySelector('#source').addEventListener('mousedown', () => {
        window.__pressed++
        const end = Date.now() + 2000
        while (Date.now() < end) {}
      })
    </script>`,
    source: `
      await page.locator('#source').waitFor()
      let result
      try { await page.dragAndDrop('#source', '#target', { timeout: 1000 }); result = 'resolved' }
      catch (error) { result = errorCategory(error) }
      await new Promise(resolve => setTimeout(resolve, 2500))
      return result
    `,
    probe:
      '({ pressed: window.__pressed, dropped: window.__events.some(e => e.type === "drop") })',
    values: ({ playwright, neo }) => {
      assert.equal(playwright, 'TimeoutError')
      assert.equal(neo, playwright)
    },
    effects: ({ playwright, neo }) => {
      assert.deepEqual(playwright, { pressed: 1, dropped: false })
      assert.deepEqual(neo, playwright)
    },
  })
  const call = outcome.calls.find((call) => call.method === 'dragAndDrop')
  assert.equal(call.result.name, 'TimeoutError')
  assert.equal(call.delivery, 'unknown', 'the target half never ran')
})
