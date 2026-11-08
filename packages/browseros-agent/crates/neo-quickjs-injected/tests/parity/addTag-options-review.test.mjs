import { test } from 'node:test'
import { compareMethod, skip } from './harness.mjs'
import { VALUES } from './values.mjs'

const html =
  '<!doctype html><p id="tag-review-target">target</p><script>window.__tagEffects=0</script>'
test('script tag option fields reject null before execution', {
  skip,
  timeout: 180000,
}, async (t) => {
  await compareMethod(t, {
    html,
    source:
      VALUES +
      `
  const run=async(receiver,options)=>outcome(async()=>{await receiver.addScriptTag(options);return 'ran'})
  return {url:await run(page,{url:null,content:'window.__tagEffects++'}),content:await run(page.mainFrame(),{url:'data:text/javascript,window.__tagEffects++',content:null}),type:await run(page,{content:'window.__tagEffects++',type:null}),effects:await page.evaluate(()=>window.__tagEffects)}
 `,
    probe: 'window.__tagEffects',
  })
})
test('style tag option fields reject null before applying styles', {
  skip,
  timeout: 180000,
}, async (t) => {
  await compareMethod(t, {
    html,
    source:
      VALUES +
      `
  const run=async(receiver,options)=>outcome(async()=>{await receiver.addStyleTag(options);return 'applied'})
  return {url:await run(page,{url:null,content:'#tag-review-target { color: rgb(1, 2, 3) }'}),content:await run(page.mainFrame(),{url:'data:text/css,%23tag-review-target{color:rgb(1,2,3)}',content:null}),color:await page.evaluate(()=>getComputedStyle(document.querySelector('#tag-review-target')).color)}
 `,
    probe:
      'getComputedStyle(document.querySelector("#tag-review-target")).color',
  })
})
