import assert from 'node:assert/strict'
import { test } from 'node:test'
import { compareFrameCases } from './frames.mjs'
import { compareMethod, skip } from './harness.mjs'

const html = `<select id="s"><option value="stay" selected>Stay</option></select><select id="multi" multiple><option value="a">A</option><option value="b">B</option></select><iframe name="child" srcdoc="<select><option id='moved' value='moved'>Moved</option></select>"></iframe><script>window.events=[];s.addEventListener('change',()=>events.push(s.value));new MutationObserver(()=>document.querySelector('browseros-neo-takeover')?.remove()).observe(document.documentElement,{childList:true,subtree:true})</script>`

test('review D: an option moved from another document remains selectable by its handle', {
  skip,
  timeout: 120000,
}, async (t) => {
  await compareMethod(t, {
    html,
    source: `await page.frameLocator('iframe').locator('#moved').waitFor({state:'attached'});
      const handle=await page.frame('child').$('#moved');
      await page.evaluate(()=>{const option=document.querySelector('iframe').contentDocument.querySelector('#moved');document.querySelector('#s').append(document.adoptNode(option));document.querySelector('#s').value='stay'});
      const outcome=await page.locator('#s').selectOption(handle,{timeout:500}).then(value=>({value}),error=>({error:error.name}));
      return {outcome,selected:await page.inputValue('#s'),inTarget:await page.locator('#s #moved').count()};`,
    probe: '({selected:s.value,events})',
    values: ({ playwright, neo }) => {
      t.diagnostic(JSON.stringify({ playwright, neo }))
      assert.deepEqual(playwright, {
        outcome: { value: ['moved'] },
        selected: 'moved',
        inTarget: 1,
      })
      assert.deepEqual(neo, playwright)
    },
  })
})

test('review D: handle lists preserve mixed-list rules and detached-option recovery', {
  skip,
  timeout: 120000,
}, async (t) => {
  await compareMethod(t, {
    html,
    source: `const a=await page.$('#multi option[value=a]');
      const b=await page.$('#multi option[value=b]');
      const settle=async value=>page.locator('#multi').selectOption(value,{timeout:250}).then(value=>({value}),error=>({error:error.name}));
      const results=[];
      for(const values of [[a,a],[a,'b'],['b',a],[{value:'b'},a]]){
        await page.locator('#multi').selectOption([]);
        results.push({outcome:await settle(values),selected:await page.$eval('#multi',s=>[...s.selectedOptions].map(o=>o.value))});
      }
      await page.evaluate(()=>{window.detached=document.querySelector('#multi option[value=b]');detached.remove()});
      const detached=await settle(b);
      await page.evaluate(()=>document.querySelector('#multi').append(window.detached));
      const recovered=await settle(b);
      return {results,detached,recovered};`,
    probe: '({selected:[...multi.selectedOptions].map(o=>o.value)})',
  })
})

test('review D: scrolled padded OOPIF boxes and hidden owners', {
  skip,
  timeout: 120000,
}, async (t) => {
  await compareFrameCases(t, [
    {
      name: 'border boxes track scrolling and become null for a hidden owner',
      source: `const owner=page.locator('#login-iframe');
      const target=page.frameLocator('#login-iframe').locator('#send');
      await owner.evaluate(e=>{e.style.border='7px solid black';e.style.padding='9px';e.style.marginTop='400px'});
      await page.evaluate(()=>window.scrollTo(0,350));
      await target.evaluate(e=>{e.style.marginTop='240px';document.body.style.height='900px';window.scrollTo(0,100)});
      const box=async()=>{const value=await target.boundingBox();return value&&Object.fromEntries(Object.entries(value).map(([k,v])=>[k,Math.round(v*100)/100]))};
      const scrolled=await box();
      await owner.evaluate(e=>e.style.display='none');
      const hidden=await target.boundingBox({timeout:1000});
      await owner.evaluate(e=>e.style.display='');
      return {scrolled,hidden,restored:await box()};`,
      expected: {
        scrolled: (box) => assert.ok(box && box.width > 0 && box.height > 0),
        restored: (box) => assert.ok(box && box.width > 0 && box.height > 0),
        hidden: null,
      },
      probe: '({})',
    },
  ])
})
