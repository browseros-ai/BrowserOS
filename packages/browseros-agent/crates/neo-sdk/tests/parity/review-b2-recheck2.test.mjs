import assert from 'node:assert/strict'
import { test } from 'node:test'
import { compareMethod, skip } from './harness.mjs'

test('review: expansion limit names its bound and leaves small dispatches usable', {
  skip,
  timeout: 120_000,
}, async (t) => {
  const result = await compareMethod(t, {
    html: `<button id="target">Target</button><output id="seen">0</output><script>target.addEventListener('ping',()=>seen.textContent=String(+seen.textContent+1));new MutationObserver(()=>document.querySelector('browseros-neo-takeover')?.remove()).observe(document.documentElement,{childList:true,subtree:true})</script>`,
    source: `const target=page.locator('#target'), shared={leaf:true};
      await target.dispatchEvent('ping',{detail:Array(10_000).fill(shared)});
      let large='resolved';
      try { await target.dispatchEvent('ping',{detail:Array(60_000).fill(shared)}) }
      catch(error) { large={name:error.name,namesLimit:error.message.includes('100000')} }
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
      assert.deepEqual(neo, {
        large: { name: 'Error', namesLimit: true },
        before: '1',
        after: '2',
      })
    },
    effects: ({ playwright, neo }) => {
      assert.equal(playwright, '3')
      assert.equal(neo, '2')
    },
  })
  t.diagnostic(JSON.stringify(result.value))
})

test('review: a shared graph below the expanded-value limit dispatches', {
  skip,
  timeout: 120_000,
}, async (t) => {
  // The eventInit object and its detail array add two values. Each expanded
  // {leaf:true} adds two more: 2 + 40_000 * 2 = 80_002, below the 100_000 cap.
  const result = await compareMethod(t, {
    html: `<button id="target">Target</button><output id="seen">0</output><script>target.addEventListener('ping',()=>seen.textContent='1');new MutationObserver(()=>document.querySelector('browseros-neo-takeover')?.remove()).observe(document.documentElement,{childList:true,subtree:true})</script>`,
    source: `const shared={leaf:true};let status,namesLimit=false;
      try { await page.locator('#target').dispatchEvent('ping',{detail:Array(40_000).fill(shared)}); status='resolved' }
      catch(error) { status=error.name;namesLimit=error.message.includes('100000') }
      return {status,namesLimit,seen:await page.locator('#seen').textContent()};`,
    probe: `document.querySelector('#seen').textContent`,
  })
  assert.deepEqual(result.value, {
    status: 'resolved',
    namesLimit: false,
    seen: '1',
  })
})
