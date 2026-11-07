// Regression: AbortSignal.any must follow transitive source signals before listeners run.
// A nested composition must also cancel a real browser action without a late click.
import assert from 'node:assert/strict'
import { test } from 'node:test'
import { compareMethod, skip } from './harness.mjs'

const html = `<button id="hidden" style="display:none" onclick="window.effects.push('hidden')">hidden</button><button id="target" onclick="window.effects.push('target')">target</button><script>window.effects=[]</script>`

for (const { name, source, expected } of [
  {
    name: 'propagates state and reason before listeners',
    source: `const root = new AbortController()
const inner = AbortSignal.any([root.signal])
const outer = AbortSignal.any([inner])
const reason = { cancelled:true }
const events = []
root.signal.addEventListener('abort', () => events.push([inner.aborted, outer.aborted]))
outer.addEventListener('abort', () => events.push(['outer']))
root.abort(reason)
return {inner:inner.aborted,outer:outer.aborted,sameReason:outer.reason===reason,events}`,
    expected: {
      inner: true,
      outer: true,
      sameReason: true,
      events: [[true, true], ['outer']],
    },
  },
  {
    name: 'prevents a canceled click from landing later',
    source: `const root = new AbortController()
const signal = AbortSignal.any([AbortSignal.any([root.signal])])
const reason = { cancelled:true }
const pending = page.locator('#hidden').click({signal,timeout:0}).then(()=>'clicked',e=>({name:e.name,cause:e.cause===reason}))
await new Promise(resolve=>setTimeout(resolve,100))
root.abort(reason)
await page.evaluate(()=>document.querySelector('#hidden').style.display='block')
const result = await pending
await page.click('#target')
return {result,effects:await page.evaluate(()=>window.effects)}`,
    expected: {
      result: { name: 'AbortError', cause: true },
      effects: ['target'],
    },
  },
]) {
  test(`nested AbortSignal.any ${name}`, { skip }, async (t) => {
    await compareMethod(t, {
      html,
      source,
      probe: () => window.effects,
      values({ playwright, neo }) {
        t.diagnostic(JSON.stringify({ playwright, neo }))
        assert.deepEqual(playwright, expected)
        assert.deepEqual(neo, playwright)
      },
    })
  })
}
