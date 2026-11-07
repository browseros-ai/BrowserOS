import { test } from 'node:test'
import { compareMethod, skip } from './harness.mjs'
import { VALUES } from './values.mjs'

const html =
  '<!doctype html><span>A</span><span>B</span><script>window.effects=0</script>'
test('querySelector strict null is a validation error', {
  skip,
  timeout: 180000,
}, async (t) => {
  await compareMethod(t, {
    html,
    source:
      VALUES +
      `return {page:await outcome(async()=>{const h=await page.$('span',{strict:null});return h?await h.evaluate(e=>e.textContent):null}),frame:await outcome(async()=>{const h=await page.mainFrame().$('span',{strict:null});return h?await h.evaluate(e=>e.textContent):null})}`,
    probe: 'window.effects',
  })
})
test('waitForFunction polling null rejects before a predicate effect', {
  skip,
  timeout: 180000,
}, async (t) => {
  await compareMethod(t, {
    html,
    source:
      VALUES +
      `return {page:await outcome(async()=> (await page.waitForFunction(()=>{window.effects++;return true},undefined,{polling:null})).jsonValue()),frame:await outcome(async()=> (await page.mainFrame().waitForFunction(()=>{window.effects++;return true},undefined,{polling:null})).jsonValue())}`,
    probe: 'window.effects',
  })
})
