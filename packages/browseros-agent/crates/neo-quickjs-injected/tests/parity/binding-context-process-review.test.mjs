// C2 finding 1, across a process round trip: the old callback's reply must never answer a
// new document's call, even when its frame returns to the original site. The parent drives
// navigation through the DOM; every assertion compares public values against isolated Playwright.
import assert from 'node:assert/strict'
import { test } from 'node:test'
import { compareMethod, skip } from './harness.mjs'

test('a pending binding reply stays with its document across an iframe process round trip', {
  skip,
  timeout: 180_000,
}, async (t) => {
  await compareMethod(t, {
    html: '<title>binding process lifetime</title><p>fixture</p>',
    isolate: true,
    source: `let releaseOld,releaseNew,oldReturned=false
await page.exposeFunction('delayed',async which=>{
  const value=await new Promise(resolve=>{if(which==='old')releaseOld=resolve;else releaseNew=resolve})
  if(which==='old')oldReturned=true
  return value
})
const address=page.url()
await page.evaluate(url=>new Promise(resolve=>{
  const frame=document.createElement('iframe');frame.id='child';frame.onload=resolve
  frame.src=url.replace('127.0.0.1','localhost');document.body.append(frame)
}),address)
const child=page.frames().find(frame=>frame.url().includes('localhost'))
if(!child)throw new Error('child did not load')
await child.evaluate(()=>{window.reply='pending';window.delayed('old').then(value=>window.reply=value).catch(()=>{})})
for(let i=0;i<500&&!releaseOld;i++)await new Promise(resolve=>setTimeout(resolve,10))
if(!releaseOld)throw new Error('old callback did not arrive')
for(const next of [address+'?bridge=1',address.replace('127.0.0.1','localhost')+'?back=1'])
  await page.evaluate(url=>new Promise(resolve=>{
    const frame=document.querySelector('#child');frame.onload=resolve;frame.src=url
  }),next)
await child.evaluate(()=>{window.reply='pending';window.delayed('new').then(value=>window.reply=value)})
for(let i=0;i<500&&!releaseNew;i++)await new Promise(resolve=>setTimeout(resolve,10))
if(!releaseNew)throw new Error('new callback did not arrive')
releaseOld('old-answer')
for(let i=0;i<500&&!oldReturned;i++)await new Promise(resolve=>setTimeout(resolve,10))
let premature=false
try{const handle=await child.waitForFunction(()=>window.reply!=='pending',undefined,{timeout:1000});await handle.dispose();premature=true}
catch(error){if(error.name!=='TimeoutError')throw error}
const beforeNew=await child.evaluate(()=>window.reply)
releaseNew('new-answer')
const handle=await child.waitForFunction(()=>window.reply!=='pending');await handle.dispose()
return {oldReturned,premature,beforeNew,result:await child.evaluate(()=>window.reply)}`,
    probe: 'document.title',
    values({ playwright, neo }) {
      t.diagnostic(JSON.stringify({ playwright, neo }))
      assert.deepEqual(playwright, {
        oldReturned: true,
        premature: false,
        beforeNew: 'pending',
        result: 'new-answer',
      })
      assert.deepEqual(neo, playwright)
    },
  })
})
