import assert from 'node:assert/strict'
import { test } from 'node:test'
import { compareMethod, skip } from './harness.mjs'

const html = `<select id="s"><option value="one">One</option><option value="two">Two</option><option value="three" selected>Three</option></select>
<iframe name="child" srcdoc="<select><option id='foreign' value='foreign'>Foreign</option></select>"></iframe>
<script>window.seen=[];for(const type of ['input','change'])s.addEventListener(type,()=>seen.push(type));new MutationObserver(()=>document.querySelector('browseros-neo-takeover')?.remove()).observe(document.documentElement,{childList:true,subtree:true})</script>`

// Since slice D (ElementHandle values served as Playwright's `elements`), Neo answers every case
// as Playwright does: compared directly, beside Playwright's own expectations.
test('review: nested, proxied, foreign and disposed handles change selection as Playwright does', {
  skip,
  timeout: 120_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html,
    source: `const h=await page.$('#s option[value=two]');
      await page.frame('child').locator('#foreign').waitFor({state:'attached'});
      const foreign=await page.frame('child').$('#foreign');
      const disposed=await page.$('#s option[value=two]');await disposed.dispose();
      const object=await page.evaluateHandle(()=>({value:'two'}));
      const cases=[['value',{value:h}],['label',{label:h}],['index',{index:h}],['proxy',new Proxy(h,{})],['foreign',foreign],['disposed',disposed],['object',object]];
      const calls=[['page',v=>page.selectOption('#s',v,{timeout:300})],['frame',v=>page.mainFrame().selectOption('#s',v,{timeout:300})],['locator',v=>page.locator('#s').selectOption(v,{timeout:300})]];
      const results=[];
      for(const [receiver,call] of calls)for(const [name,value] of cases){
        await page.evaluate(()=>{s.value='three';seen=[]});
        let rejected=false;try{await call(value)}catch{rejected=true}
        results.push({receiver,name,rejected,selected:await page.inputValue('#s'),events:await page.evaluate(()=>seen)});
      }
      await page.evaluate(()=>{s.value='three';seen=[]});return results;`,
    probe: `({selected:s.value,events:seen})`,
    values: ({ playwright, neo }) => {
      assert.equal(neo.length, 21)
      assert.deepEqual(neo, playwright, 'every receiver and case as Playwright')
      for (const row of playwright) {
        if (['proxy', 'object'].includes(row.name)) {
          assert.equal(row.rejected, false, JSON.stringify(row))
          assert.equal(row.selected, row.name === 'proxy' ? 'two' : 'one')
        } else {
          assert.equal(row.rejected, true, JSON.stringify(row))
          assert.equal(row.selected, 'three')
          assert.deepEqual(row.events, [])
        }
      }
    },
  })
  // Per receiver, the proxied, foreign, disposed and plain-JSHandle values reach the runtime, as
  // they reach Playwright's server; the three handles nested in option fields fail the
  // protocol's validation before anything is sent.
  assert.equal(
    outcome.calls.filter((c) => c.method === 'selectOption').length,
    12,
    'selectOption calls sent',
  )
})

test('review: a plain guid object follows plain option-object semantics', {
  skip,
  timeout: 120_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html,
    source: `const h=await page.$('#s option[value=two]');
      return {result:await page.locator('#s').selectOption({_guid:h._guid}),selected:await page.inputValue('#s')};`,
    probe: `({selected:s.value,events:seen})`,
  })
  assert.deepEqual(outcome.value, { result: ['one'], selected: 'one' })
})

// As first written by the F1 reviewer: an array proxy whose own methods lie selects 'two' through
// Neo as through Playwright, now that `elements` is served (slice D).
test('review: an array proxy of a handle selects its option as Playwright does', {
  skip,
  timeout: 120_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html,
    source: `const h=await page.$('#s option[value=two]');
      const values=new Proxy([h],{get(target,key,receiver){return key==='some'?()=>false:Reflect.get(target,key,receiver)}});
      const results=[];
      for(const call of [()=>page.selectOption('#s',values),()=>page.mainFrame().selectOption('#s',values),()=>page.locator('#s').selectOption(values)]){
        await page.evaluate(()=>{s.value='three';seen=[]});
        let rejected=false;try{await call()}catch{rejected=true}
        results.push({rejected,selected:await page.inputValue('#s'),events:await page.evaluate(()=>seen)});
      }
      await page.evaluate(()=>{s.value='three';seen=[]});return results;`,
    probe: `({selected:s.value,events:seen})`,
    values: ({ playwright, neo }) => {
      assert.ok(
        playwright.every((row) => !row.rejected && row.selected === 'two'),
        JSON.stringify(playwright),
      )
      assert.deepEqual(neo, playwright, 'every receiver as Playwright')
    },
  })
  assert.equal(
    outcome.calls.filter((c) => c.method === 'selectOption').length,
    3,
    'one selectOption call per receiver',
  )
})
