// Context init scripts must reach reload and a new page initial document.
import { test } from 'node:test'
import { compareMethod, skip } from './harness.mjs'

const html =
  '<!doctype html><script>window.pageSaw = window.earlyInit ?? null</script><p>init</p>'
test('page init script before reload control', {
  skip,
  timeout: 180000,
}, async (t) => {
  await compareMethod(t, {
    html,
    source: `await page.addInitScript(()=>{window.earlyInit='ready'});await page.reload();return await page.evaluate(()=>({init:window.earlyInit??null,pageSaw:window.pageSaw}))`,
    probe: 'window.pageSaw',
  })
})
test('context init script runs before reload', {
  skip,
  timeout: 180000,
}, async (t) => {
  await compareMethod(t, {
    html,
    source: `const script=await page.context().addInitScript(()=>{window.earlyInit='ready'});try{await page.reload();return await page.evaluate(()=>({init:window.earlyInit??null,pageSaw:window.pageSaw}))}finally{await script.dispose()}`,
    probe: 'window.pageSaw',
  })
})
test('context init script reaches a new page initial document', {
  skip,
  timeout: 180000,
}, async (t) => {
  await compareMethod(t, {
    html,
    source: `const browserContext=page.context();const script=await browserContext.addInitScript(()=>{window.earlyInit='ready'});const second=await browserContext.newPage();try{return await second.evaluate(()=>window.earlyInit??null)}finally{await second.close();await script.dispose()}`,
    probe: 'window.pageSaw',
  })
})
