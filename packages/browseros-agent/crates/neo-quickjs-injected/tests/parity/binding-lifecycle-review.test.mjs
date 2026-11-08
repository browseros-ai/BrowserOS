// Independent C2 lifecycle parity: a reply stays with its document, and a binding call keeps
// its source frame after detach. Invalid names, rejected callbacks and OOPIF navigation are
// positive controls. Each engine owns its browser so context-wide bindings cannot cross-answer.
import assert from 'node:assert/strict'
import { test } from 'node:test'
import { compareMethod, skip } from './harness.mjs'

const html = '<title>binding review</title><p id="target">target</p>'
for (const [name, expose] of [
  [
    'page.exposeFunction',
    "await page.exposeFunction('delayed', value => answer(value))",
  ],
  [
    'context.exposeBinding',
    "await page.context().exposeBinding('delayed', (_, value) => answer(value))",
  ],
]) {
  test(`review C2: ${name} never delivers an old document's reply into a new call`, {
    skip,
    timeout: 180_000,
  }, async (t) => {
    await compareMethod(t, {
      html,
      isolate: true,
      source: `let releaseOld, releaseNew
let oldReturned = false
async function answer(which) {
  const value = await new Promise(resolve => { if (which === 'old') releaseOld = resolve; else releaseNew = resolve })
  if (which === 'old') oldReturned = true
  return value
}
${expose}
await page.evaluate(() => { window.bindingResult = 'pending'; window.delayed('old').then(value => window.bindingResult = value).catch(() => {}) })
for (let i=0; i<500 && !releaseOld; i++) await new Promise(resolve => setTimeout(resolve,10))
if (!releaseOld) throw new Error('old callback did not arrive')
await page.goto(page.url().split('?')[0] + '?new-document=1')
await page.evaluate(() => { window.bindingResult = 'pending'; window.delayed('new').then(value => window.bindingResult = value) })
for (let i=0; i<500 && !releaseNew; i++) await new Promise(resolve => setTimeout(resolve,10))
if (!releaseNew) throw new Error('new callback did not arrive')
releaseOld('old-answer')
for (let i=0; i<500 && !oldReturned; i++) await new Promise(resolve => setTimeout(resolve,10))
let premature = false
try {
  const handle = await page.waitForFunction(() => window.bindingResult !== 'pending', undefined, { timeout: 1000 })
  await handle.dispose()
  premature = true
} catch (error) { if (error.name !== 'TimeoutError') throw error }
const beforeNew = await page.evaluate(() => window.bindingResult)
releaseNew('new-answer')
const handle = await page.waitForFunction(() => window.bindingResult !== 'pending')
await handle.dispose()
return { oldReturned, premature, beforeNew, result: await page.evaluate(() => window.bindingResult) }`,
      probe: 'window.bindingResult',
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
}

test('review C2: invalid names reject without exposing a page function', {
  skip,
  timeout: 180_000,
}, async (t) => {
  await compareMethod(t, {
    html,
    isolate: true,
    source: `const results=[]
const browserContext=page.context()
for (const [receiver,method] of [[page,'exposeFunction'],[page,'exposeBinding'],[browserContext,'exposeFunction'],[browserContext,'exposeBinding']])
  results.push(await receiver[method](42,()=>1).then(()=>'resolved',error=>error.name))
return {results,exposed:await page.evaluate(()=>typeof window['42'])}`,
    probe: "document.querySelector('#target').textContent",
    values({ playwright, neo }) {
      t.diagnostic(JSON.stringify({ playwright, neo }))
      assert.ok(playwright.results.every((result) => result !== 'resolved'))
      assert.equal(playwright.exposed, 'undefined')
      assert.deepEqual(neo, playwright)
    },
  })
})

test('review C2: context binding identifies an OOPIF first script across a process-changing navigation', {
  skip,
  timeout: 240_000,
}, async (t) => {
  await compareMethod(t, {
    html: `<title>binding source</title><script>
if (location.search.startsWith('?child='))
  window.answer = window.fromFrame(location.search).then(value => ({value}), error => ({error:error.name}));
</script>`,
    isolate: true,
    source: `const browserContext=page.context()
const seen=[]
await browserContext.exposeBinding('fromFrame',(source,phase)=>{
  seen.push({frame:source.frame,phase})
  return {phase,context:source.context===browserContext,page:source.page===page,child:source.frame!==page.mainFrame()}
})
const url=page.url()
await page.evaluate(url=>{
  const frame=document.createElement('iframe')
  frame.src=url.replace('127.0.0.1','localhost')+'?child=one'
  document.body.append(frame)
},url)
for(let i=0;i<500 && seen.length<1;i++) await new Promise(resolve=>setTimeout(resolve,10))
if(seen.length!==1) throw new Error('first inline callback did not arrive')
const child=page.frames().find(frame=>frame.url().includes('localhost'))
if(!child) throw new Error('OOPIF not found')
const first=await child.evaluate(()=>window.answer)
await page.evaluate(url=>new Promise(resolve=>{
  const child=document.querySelector('iframe')
  child.onload=resolve
  child.src=url+'?child=two'
}),url)
const second=await child.evaluate(()=>window.answer)
return {first,second,frames:seen.map(record=>record.frame===child),phases:seen.map(record=>record.phase)}`,
    probe: 'document.title',
    values({ playwright, neo }) {
      t.diagnostic(JSON.stringify({ playwright, neo }))
      assert.deepEqual(playwright, {
        first: {
          value: {
            phase: '?child=one',
            context: true,
            page: true,
            child: true,
          },
        },
        second: {
          value: {
            phase: '?child=two',
            context: true,
            page: true,
            child: true,
          },
        },
        frames: [true, true],
        phases: ['?child=one', '?child=two'],
      })
      assert.deepEqual(neo, playwright)
    },
  })
})

test('review C2: rejected callbacks preserve thrown values and error categories', {
  skip,
  timeout: 180_000,
}, async (t) => {
  await compareMethod(t, {
    html,
    isolate: true,
    source: `await page.exposeFunction('throwsValue',async()=>{throw {tag:'value',nil:null,missing:undefined,nan:NaN}})
await page.context().exposeBinding('throwsError',async()=>{throw new TypeError('callback rejected')})
return await page.evaluate(async()=>{
  let thrown, error
  try {await window.throwsValue()} catch(value) {thrown={tag:value.tag,nil:value.nil,missing:typeof value.missing,nan:Number.isNaN(value.nan)}}
  try {await window.throwsError()} catch(value) {error={name:value.name,isError:value instanceof Error}}
  return {thrown,error}
})`,
    probe: 'document.title',
    values({ playwright, neo }) {
      t.diagnostic(JSON.stringify({ playwright, neo }))
      assert.deepEqual(playwright, {
        thrown: { tag: 'value', nil: null, missing: 'undefined', nan: true },
        error: { name: 'TypeError', isError: true },
      })
      assert.deepEqual(neo, playwright)
    },
  })
})

test('review C2: a binding call retains its source frame after that frame detaches', {
  skip,
  timeout: 180_000,
}, async (t) => {
  await compareMethod(t, {
    html,
    isolate: true,
    source: `let seen
await page.exposeBinding('departing',source=>{seen=source;return 'answered'})
await page.evaluate(()=>new Promise(resolve=>{
  const child=document.createElement('iframe')
  child.src=location.pathname+'?detaching-child=1'
  child.onload=resolve
  document.body.append(child)
}))
const child=page.frames().find(frame=>frame.url().includes('detaching-child'))
if(!child) throw new Error('child did not load')
await page.evaluate(()=>{
  const child=document.querySelector('iframe')
  child.contentWindow.departing().catch(()=>{})
  child.remove()
})
for(let i=0;i<500 && !seen;i++) await new Promise(resolve=>setTimeout(resolve,10))
if(!seen) throw new Error('departing callback did not arrive')
return {sameChild:seen.frame===child,main:seen.frame===page.mainFrame(),page:seen.page===page,context:seen.context===page.context()}`,
    probe: 'document.title',
    values({ playwright, neo }) {
      t.diagnostic(JSON.stringify({ playwright, neo }))
      assert.deepEqual(playwright, {
        sameChild: true,
        main: false,
        page: true,
        context: true,
      })
      assert.deepEqual(neo, playwright)
    },
  })
})
