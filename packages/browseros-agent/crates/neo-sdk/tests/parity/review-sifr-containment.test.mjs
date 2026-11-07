// Scoped re-check: path spelling must not turn a failed upload into a successful one.
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { test } from 'node:test'
import { withEngines } from './capture.mjs'
import { skip } from './harness.mjs'

test('a trailing slash cannot turn an invalid write into a file overwrite', {
  skip,
  timeout: 120_000,
}, async (t) => {
  await withEngines(
    t,
    { pages: { '/write.html': '<h1>Write boundary</h1>' } },
    async (run) => {
      const result = await run(
        '/write.html',
        `
      return await page.screenshot({path:at('capture.png/'),type:'png'}).then(()=> 'resolved',e=>e.name);
    `,
        undefined,
        { 'capture.png': 'original' },
      )
      const original = Buffer.from('original')
      const files = {
        playwrightUnchanged: readFileSync(
          result.playwright.file('capture.png'),
        ).equals(original),
        neoUnchanged: readFileSync(result.neo.file('capture.png')).equals(
          original,
        ),
      }
      t.diagnostic(
        JSON.stringify({
          playwright: result.playwright.value,
          neo: result.neo.value,
          ...files,
        }),
      )
      assert.equal(result.playwright.value, 'Error')
      assert.equal(files.playwrightUnchanged, true)
      assert.equal(result.neo.value, result.playwright.value)
      assert.equal(files.neoUnchanged, true)
    },
  )
})

test('a trailing slash on a file path preserves refusal and the prior file list', {
  skip,
  timeout: 120_000,
}, async (t) => {
  const html = `<input id="file" type="file"><script>
    window.events=[];for(const name of ['input','change']) document.querySelector('input').addEventListener(name,()=>events.push(name));
    window.facts=()=>({events:[...events],files:[...document.querySelector('input').files].map(f=>f.name)});
  </script>`
  await withEngines(t, { pages: { '/slash.html': html } }, async (run) => {
    const result = await run(
      '/slash.html',
      `
      const bytes=text=>typeof Buffer==='function'?Buffer.from(text):Uint8Array.from(text,c=>c.charCodeAt(0));
      const results={};
      for (const path of ['real.txt/','real.txt/.']) {
        await page.locator('#file').setInputFiles({name:'kept.txt',buffer:bytes('kept')});
        await page.evaluate(()=>window.events=[]);
        const before=await page.evaluate(()=>facts());
        const upload=await page.locator('#file').setInputFiles(at(path)).then(()=> 'resolved',e=>e.name);
        results[path]={before,upload,after:await page.evaluate(()=>facts())};
      }
      return results;
    `,
      'facts()',
      { 'real.txt': 'real' },
    )
    assert.equal(result.playwright.error, undefined)
    assert.equal(result.neo.error, undefined)
    t.diagnostic(
      JSON.stringify({
        playwright: result.playwright.value,
        neo: result.neo.value,
      }),
    )
    for (const path of ['real.txt/', 'real.txt/.']) {
      assert.equal(result.playwright.value[path].upload, 'Error')
      assert.deepEqual(
        result.playwright.value[path].after,
        result.playwright.value[path].before,
      )
    }
    assert.deepEqual(result.neo.value, result.playwright.value)
    assert.deepEqual(result.neo.probe, result.playwright.probe)
  })
})
