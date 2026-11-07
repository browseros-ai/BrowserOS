// B1 regressions, every case compared with Playwright. Since slice S an aborted signal on a
// screenshot rejects AbortError before any browser effect. page.pdf ignores a signal as
// Playwright's does, whose print carries none (`kNoTimeout`), so it prints (med-pw-eval DECIDED,
// B2 review). `signal: null` is no signal (DECIDED 01:27).
import assert from 'node:assert/strict'
import { test } from 'node:test'
import { withEngines } from './capture.mjs'
import { skip } from './harness.mjs'

const html = `<!doctype html><input style="caret-color:red"><div id=box style="width:80px;height:40px"></div><div style="height:2000px"></div><div id=far style="width:80px;height:40px"></div><script>window.prints=0;window.styleAdds=0;addEventListener('beforeprint',()=>window.prints++);new MutationObserver(rs=>{for(const r of rs)for(const n of r.addedNodes)if(n.nodeName==='STYLE')window.styleAdds++}).observe(document.documentElement,{childList:true,subtree:true})</script>`
const probe = `({prints:window.prints,styleAdds:window.styleAdds,styles:document.querySelectorAll('style').length,caret:document.querySelector('input').style.caretColor,scrollY,masks:document.querySelectorAll('x-pw-glass').length})`
test('B1 independent option probes', { skip, timeout: 600000 }, async (t) => {
  await withEngines(t, { pages: { '/case': html } }, async (run) => {
    for (const [name, source] of [
      [
        'page signal',
        `try {await page.screenshot({signal:{aborted:true},style:'body{color:red}',path:at('signal.png')});return 'captured'}catch(e){return {name:e.name,signal:/signal/.test(e.message)}}`,
      ],
      [
        'locator signal',
        `try {await page.locator('#far').screenshot({signal:{aborted:true},style:'body{color:red}',path:at('signal.png')});return 'captured'}catch(e){return {name:e.name,signal:/signal/.test(e.message)}}`,
      ],
      [
        'page null signal',
        `const b=await page.screenshot({signal:null,clip:{x:0,y:0,width:20,height:10}});const d=describeImage(b);return {type:d.type,width:d.width,height:d.height}`,
      ],
      [
        'locator null signal',
        `const b=await page.locator('#box').screenshot({signal:null});const d=describeImage(b);return {type:d.type,width:d.width,height:d.height}`,
      ],
      [
        'pdf null signal',
        `const p=describePdf(await page.pdf({signal:null,pageRanges:'1'}));return {header:p.header,pages:p.pages,mediaBox:p.mediaBox}`,
      ],
      [
        'pdf signal',
        `try {await page.pdf({signal:{aborted:true},path:at('signal.pdf')});return 'printed'}catch(e){return {name:e.name,signal:/signal/.test(e.message)}}`,
      ],
      [
        'zero clip',
        `try {await page.screenshot({clip:{x:0,y:0,width:0,height:10}});return 'captured'}catch(e){return {name:e.name}}`,
      ],
      [
        'locator invalid quality',
        `try {await page.locator('#box').screenshot({type:'jpeg',quality:101});return 'captured'}catch(e){return {name:e.name}}`,
      ],
      [
        'invalid pdf ranges',
        `try {await page.pdf({pageRanges:'2-1'});return 'printed'}catch(e){return {name:e.name}}`,
      ],
      [
        'empty pdf format',
        `const b=await page.pdf({format:'',width:480,height:480,pageRanges:'1'});const p=describePdf(b);return {pages:p.pages,mediaBox:p.mediaBox}`,
      ],
      [
        'pdf format precedence',
        `const b=await page.pdf({format:'A4',width:'invalid',height:'invalid',pageRanges:'1'});const p=describePdf(b);return {pages:p.pages,mediaBox:p.mediaBox}`,
      ],
    ])
      await t.test(name, async () => {
        const r = await run('/case', source, probe)
        const record = Object.fromEntries(
          Object.entries(r).map(([k, v]) => [
            k,
            {
              value: v.value,
              error: v.error,
              probe: v.probe,
              calls: v.outcome?.calls?.map((c) => c.method),
              artifacts: v.outcome?.artifacts,
            },
          ]),
        )
        t.diagnostic(JSON.stringify({ name, ...record }))
        assert.deepEqual(r.neo.value, r.playwright.value)
        assert.deepEqual(r.neo.probe, r.playwright.probe)
      })
  })
})
