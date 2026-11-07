// B2 review contract: page.pdf ignores signal, as pinned Playwright does.
// Each engine first prints a control, then compares PDF facts and actual print/file effects.
import assert from 'node:assert/strict'
import { test } from 'node:test'
import { withEngines } from './capture.mjs'
import { skip } from './harness.mjs'

for (const aborted of [false, true])
  test('page.pdf ignores ' +
    (aborted ? 'an already-aborted' : 'a live') +
    ' signal', { skip, timeout: 180000 }, async (t) => {
    await withEngines(
      t,
      {
        pages: {
          '/case': `<!doctype html><p>PDF signal</p><script>window.prints=0;addEventListener('beforeprint',()=>window.prints++)</script>`,
        },
      },
      async (run) => {
        const result = await run(
          '/case',
          `const facts=bytes=>{const {header,pages,mediaBox}=describePdf(bytes);return {header,pages,mediaBox}}
const options={width:120,height:80,pageRanges:'1'}
const control=facts(await page.pdf({...options,path:at('control.pdf')}))
const controller=new AbortController()
if(${aborted})controller.abort('already aborted')
let signalled
try {signalled=facts(await page.pdf({...options,signal:controller.signal,path:at('signal.pdf')}))}
catch(error){signalled={error:error.name}}
return {control,signalled}`,
          'window.prints',
        )
        const { playwright, neo } = result
        t.diagnostic(
          JSON.stringify({
            aborted,
            playwright: {
              value: playwright.value,
              error: playwright.error,
              prints: playwright.probe,
              wrote: !!playwright.file('signal.pdf'),
            },
            neo: {
              value: neo.value,
              error: neo.error,
              prints: neo.probe,
              wrote: !!neo.file('signal.pdf'),
              calls: neo.outcome.calls.map((c) => c.method),
            },
          }),
        )
        assert.equal(playwright.error, undefined)
        assert.equal(neo.error, undefined)
        assert.equal(playwright.value.control.header, '%PDF-')
        assert.equal(playwright.value.control.pages, 1)
        assert.deepEqual(
          neo.value.control,
          playwright.value.control,
          'both engines can print the control',
        )
        assert.deepEqual(
          playwright.value.signalled,
          playwright.value.control,
          'Playwright ignores the signal',
        )
        assert.equal(playwright.probe, 2)
        assert.ok(playwright.file('signal.pdf'))
        assert.deepEqual(
          neo.value.signalled,
          playwright.value.signalled,
          'the signal does not change the PDF result',
        )
        assert.equal(
          neo.probe,
          playwright.probe,
          'the signalled print takes place',
        )
        assert.ok(
          neo.file('signal.pdf'),
          'the signalled print writes its artifact',
        )
      },
    )
  })
