// `isVisible` and `isHidden` take Playwright's `TimeoutOptions` in the pin's client types, but
// Playwright sends both with `kNoTimeout` (`client/frame.ts`), so they read no `signal`: an
// already-aborted one is ignored and the read runs. Neo's reads do the same on every receiver
// (Locator, Page, Frame), and neither engine touches the signal. Compared directly.
//
// Runs under Node, not Bun, on the shared harness (harness.mjs).
import assert from 'node:assert/strict'
import { test } from 'node:test'
import { compareMethod, skip } from './harness.mjs'

const html = '<button id="target">Target</button>'

const source = `
  const touched = []
  const signal = {
    aborted: true,
    reason: 'stop',
    addEventListener() { touched.push('addEventListener') },
    removeEventListener() { touched.push('removeEventListener') },
  }
  const observed = []
  for (const [family, target] of [['locator', page.locator('#target')], ['page', page], ['frame', page.mainFrame()]]) {
    for (const method of ['isVisible', 'isHidden']) {
      const options = { signal }
      let value, error
      try { value = await target[method](...(family === 'locator' ? [options] : ['#target', options])) }
      catch (caught) { error = caught }
      observed.push({ name: family + '.' + method, value: value ?? null, error: error ? error.name : null })
    }
  }
  return { observed, touched }
`

test('visibility reads ignore a signal in both engines, as Playwright sends them with kNoTimeout', {
  skip,
  timeout: 180_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html,
    source,
    probe: 'document.querySelector("#target").textContent',
  })
  assert.deepEqual(outcome.value, {
    observed: [
      { name: 'locator.isVisible', value: true, error: null },
      { name: 'locator.isHidden', value: false, error: null },
      { name: 'page.isVisible', value: true, error: null },
      { name: 'page.isHidden', value: false, error: null },
      { name: 'frame.isVisible', value: true, error: null },
      { name: 'frame.isHidden', value: false, error: null },
    ],
    touched: [],
  })
  assert.deepEqual(
    outcome.calls.map((call) => call.method),
    ['isVisible', 'isHidden', 'isVisible', 'isHidden', 'isVisible', 'isHidden'],
    'Neo sent each read',
  )
})
