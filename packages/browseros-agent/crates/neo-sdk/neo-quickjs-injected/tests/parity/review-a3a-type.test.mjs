import assert from 'node:assert/strict'
import { test } from 'node:test'
import { compareMethod, skip } from './harness.mjs'

const page = `<input id="name" value="abcd"><input id="empty" value="keep"><input id="sink" value="sink">
 <input id="hidden" value="hidden" style="display:none"><input id="detach" onfocus="this.remove()">
 <button id="caret" onclick="const e=document.querySelector('#name');e.focus();e.setSelectionRange(2,2)">Caret</button>
 <button id="sink-focus" onclick="const e=document.querySelector('#sink');e.focus();e.setSelectionRange(0,0)">Sink</button>
 <iframe name="child" srcdoc="<input id='child-input' value='old'>"></iframe>
 <script>window.events=[];for(const type of ['keydown','keypress','input','keyup'])addEventListener(type,e=>events.push({type,target:e.target.id,key:e.key??null,data:e.data??null}),true)</script>`

test('focused typing keeps the caret, unicode inserts text, and empty typing only focuses', {
  skip,
  timeout: 120_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html: page,
    source: `await page.locator('#caret').click();await page.locator('#name').pressSequentially('é😀z',{delay:10});
   await page.locator('#empty').type('');return {name:await page.locator('#name').inputValue(),empty:await page.locator('#empty').inputValue()};`,
    probe: `({value:document.querySelector('#name').value,focused:document.activeElement.id,caret:document.querySelector('#empty').selectionStart,events})`,
  })
  assert.deepEqual(outcome.value, { name: 'abé😀zcd', empty: 'keep' })
  const calls = outcome.calls.filter((c) => c.method === 'type')
  assert.equal(calls.length, 2)
  assert.equal(calls[0].delivery, 'done')
  assert.equal(calls[1].delivery, 'notSent')
})

test('hidden and focus-detached targets behave like Playwright and child Frame.type works', {
  skip,
  timeout: 120_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html: page,
    source: `await page.locator('#sink-focus').click();const results=[];
   for(const id of ['hidden','detach']){try{await page.locator('#'+id).pressSequentially('xy',{timeout:500});results.push('resolved')}catch(e){results.push(errorCategory(e))}}
   const child=page.frame('child');await child.locator('#child-input').waitFor();await child.type('#child-input','new');
   return {results,hidden:await page.locator('#hidden').inputValue(),sink:await page.locator('#sink').inputValue(),detached:await page.locator('#detach').count(),child:await child.locator('#child-input').inputValue()};`,
    probe: `({hidden:document.querySelector('#hidden').value,sink:document.querySelector('#sink').value,detached:!document.querySelector('#detach'),child:document.querySelector('iframe').contentDocument.querySelector('#child-input').value,events})`,
  })
  assert.equal(outcome.value.detached, 0)
  assert.equal(outcome.value.child, 'newold')
})
