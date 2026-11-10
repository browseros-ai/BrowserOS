import assert from 'node:assert/strict'
import { test } from 'node:test'
import { compareMethod, skip } from './harness.mjs'

test('review: large and small shared graphs both dispatch', {
  skip,
  timeout: 120_000,
}, async (t) => {
  const result = await compareMethod(t, {
    html: `<button id="target">Target</button><output id="seen">0</output><script>target.addEventListener('ping',()=>seen.textContent=String(+seen.textContent+1));new MutationObserver(()=>document.querySelector('browseros-neo-takeover')?.remove()).observe(document.documentElement,{childList:true,subtree:true})</script>`,
    source: `const target=page.locator('#target'), shared={leaf:true};
      await target.dispatchEvent('ping',{detail:Array(10_000).fill(shared)});
      let large='resolved';
      try { await target.dispatchEvent('ping',{detail:Array(60_000).fill(shared)}) }
      catch(error) { large=error.name }
      const before=await page.locator('#seen').textContent();
      await target.dispatchEvent('ping',{});
      return {large,before,after:await page.locator('#seen').textContent()};`,
    probe: `document.querySelector('#seen').textContent`,
    values: ({ playwright, neo }) => {
      assert.deepEqual(playwright, {
        large: 'resolved',
        before: '2',
        after: '3',
      })
      assert.deepEqual(neo, playwright)
    },
    effects: ({ playwright, neo }) => {
      assert.equal(playwright, '3')
      assert.equal(neo, playwright)
    },
  })
  t.diagnostic(JSON.stringify(result.value))
})

test('review: a graph with 40,000 shared references dispatches', {
  skip,
  timeout: 120_000,
}, async (t) => {
  const result = await compareMethod(t, {
    html: `<button id="target">Target</button><output id="seen">0</output><script>target.addEventListener('ping',()=>seen.textContent='1');new MutationObserver(()=>document.querySelector('browseros-neo-takeover')?.remove()).observe(document.documentElement,{childList:true,subtree:true})</script>`,
    source: `const shared={leaf:true};let status,rejected=false;
      try { await page.locator('#target').dispatchEvent('ping',{detail:Array(40_000).fill(shared)}); status='resolved' }
      catch(error) { status=error.name;rejected=true }
      return {status,rejected,seen:await page.locator('#seen').textContent()};`,
    probe: `document.querySelector('#seen').textContent`,
  })
  assert.deepEqual(result.value, {
    status: 'resolved',
    rejected: false,
    seen: '1',
  })
})
