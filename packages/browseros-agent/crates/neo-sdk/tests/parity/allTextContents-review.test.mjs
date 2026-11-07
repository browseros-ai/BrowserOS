import { test } from 'node:test'
import { compareMethod, skip } from './harness.mjs'
import { VALUES } from './values.mjs'

test('allTextContents keeps the same main-world getter result as $$eval', {
  skip,
  timeout: 180000,
}, async (t) => {
  await compareMethod(t, {
    html: '<!doctype html><span>A</span><span>B</span><script>window.reads=0</script>',
    source:
      VALUES +
      `await page.evaluate(()=>Object.defineProperty(document.querySelector('span'),'textContent',{get(){window.reads++;return 42}}));return {direct:await outcome(()=>page.$$eval('span',nodes=>nodes.map(n=>n.textContent||''))),collection:await outcome(()=>page.locator('span').allTextContents())}`,
    probe: 'window.reads',
  })
})
