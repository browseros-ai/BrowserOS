// Independent A1 review regressions. The detached primitive and ignored signal cases
// fail on 55c5d07b2; rejecting unsupported signal is acceptable until cancellation is served.
import { test } from 'node:test'
import { compareMethod, skip } from './harness.mjs'

const html =
  '<title>Review</title><p id="p">text</p><iframe name="child" srcdoc="<p>child</p>"></iframe>'
const settle = `const settle = async (f) => { try { return { value: await f() } } catch (e) { return { error: errorCategory(e) } } };`

test('invalid inputs on each evaluate and handle method preserve rejection categories', {
  skip,
  timeout: 180000,
}, async (t) => {
  await compareMethod(t, {
    html,
    source: `${settle}
 const h = await page.evaluateHandle(() => ({ n: 1 }))
 const f = page.mainFrame()
 const results = {
   pageEvaluate: await settle(() => page.evaluate('!@#')),
   pageEvaluateHandle: await settle(() => page.evaluateHandle(x => x, () => {})),
   frameEvaluate: await settle(() => f.evaluate('!@#')),
   frameEvaluateHandle: await settle(() => f.evaluateHandle(x => x, () => {})),
   locatorEvaluate: await settle(() => page.locator('[').evaluate(e => e.id, undefined, {timeout:500})),
   handleEvaluate: await settle(() => h.evaluate('!@#')),
   handleEvaluateHandle: await settle(() => h.evaluateHandle((o,x) => x, () => {})),
 }
 await h.dispose()
 results.disposedJsonValue = await settle(() => h.jsonValue())
 results.disposedEvaluate = await settle(() => h.evaluate(o => o.n))
 results.disposeAgain = await settle(() => h.dispose().then(() => 'ok'))
 return results
 `,
    probe: 'document.querySelector("#p").textContent',
  })
})

test('locator.evaluate preserves null undefined numbers dates regexp map and bigint', {
  skip,
  timeout: 180000,
}, async (t) => {
  await compareMethod(t, {
    html,
    source: `
 const describe = x => x === undefined ? 'undefined' : x === null ? 'null' : Object.is(x,-0) ? '-0' : typeof x === 'number' && Number.isNaN(x) ? 'NaN' : typeof x === 'bigint' ? String(x)+'n' : x instanceof Date ? {date:x.toISOString()} : x instanceof RegExp ? {regexp:String(x)} : x instanceof Map ? {map:[...x]} : x
 const vals = []
 for (const expression of ['null','undefined','-0','NaN',"new Date('2020-01-01Z')",'/abc/gi','new Map([[1,2]])','42n'])
   vals.push(describe(await page.locator('#p').evaluate('() => ('+expression+')')))
 return vals`,
    probe: 'document.querySelector("#p").textContent',
  })
})

test('a primitive handle keeps jsonValue after its iframe is removed', {
  skip,
  timeout: 180000,
}, async (t) => {
  await compareMethod(t, {
    html,
    source: `${settle}
 await page.frameLocator('iframe').locator('p').waitFor({state:'attached'})
 const frame = page.frames().find(f => f.name() === 'child')
 const primitive = await frame.evaluateHandle(() => 42)
 const object = await frame.evaluateHandle(() => ({n:42}))
 await page.evaluate(() => document.querySelector('iframe').remove())
 return { primitive: await settle(() => primitive.jsonValue()), object: await settle(() => object.jsonValue()) }
 `,
    probe: 'document.querySelectorAll("iframe").length',
  })
})

test('locator.evaluate supports or refuses signal before changing the page', {
  skip,
  timeout: 180000,
}, async (t) => {
  await compareMethod(t, {
    html,
    source: `${settle}
 const signal = { aborted: true, reason: 'stop', addEventListener() {}, removeEventListener() {} }
 const rejected = await page.locator('#p').evaluate(e => {e.textContent='unexpected';return 'ran'}, undefined, {signal,timeout:500}).then(() => false, error => error instanceof Error)
 return {rejected,text:await page.evaluate(() => document.querySelector('#p').textContent)}
 `,
    probe: 'document.querySelector("#p").textContent',
  })
})
