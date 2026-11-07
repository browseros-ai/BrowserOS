import assert from 'node:assert/strict'
import { test } from 'node:test'
import { compareMethod, skip } from './harness.mjs'

const html = `<select id="s"><option value="one">One</option><option value="two">Two</option><option value="three" selected>Three</option></select>
<iframe name="child" srcdoc="<select><option id='foreign' value='foreign'>Foreign</option></select>"></iframe>
<script>window.seen=[];for(const type of ['input','change'])s.addEventListener(type,()=>seen.push(type));new MutationObserver(()=>document.querySelector('browseros-neo-takeover')?.remove()).observe(document.documentElement,{childList:true,subtree:true})</script>`

test('review: nested, proxied, foreign and disposed handles cannot change selection', {
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
      for (const row of neo) {
        assert.equal(row.rejected, true, JSON.stringify(row))
        assert.equal(row.selected, 'three')
        assert.deepEqual(row.events, [])
      }
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
  assert.equal(
    outcome.calls.filter((c) => c.method === 'selectOption').length,
    0,
    'SDK refusals send no selectOption call',
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

// Adapted (pw-dom-reads, F1 fix): as written this case required Neo to select 'two', which only
// serving `elements` could do. Each call's rejection is now caught, as the first case does, and
// Neo's refusal and Playwright's selection are both asserted.
test('review: an array proxy cannot bypass the handle refusal', {
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
      assert.equal(neo.length, 3)
      for (const row of neo) {
        assert.equal(row.rejected, true, JSON.stringify(row))
        assert.equal(row.selected, 'three')
        assert.deepEqual(row.events, [])
      }
    },
  })
  assert.equal(
    outcome.calls.filter((c) => c.method === 'selectOption').length,
    0,
    'SDK refusals send no selectOption call',
  )
})
