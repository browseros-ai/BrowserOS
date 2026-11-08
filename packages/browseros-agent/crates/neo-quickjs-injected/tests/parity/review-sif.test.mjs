// Independent SIF checks: frame routing, atomic refusals, cancellation, and drop data.
import assert from 'node:assert/strict'
import { test } from 'node:test'
import { withEngines } from './capture.mjs'
import { skip } from './harness.mjs'

test('a replacement during input does not receive the file list twice', {
  skip,
  timeout: 120_000,
}, async (t) => {
  const html = `<input id="file" type="file"><script>
    window.events=[];
    const input=document.querySelector('input');
    input.addEventListener('input',()=>{
      events.push('input');window.saved=input;
      const replacement=document.createElement('input');replacement.type='file';replacement.id='file';input.replaceWith(replacement);
    });
    input.addEventListener('change',()=>events.push('change'));
  </script>`
  await withEngines(t, { pages: { '/replace.html': html } }, async (run) => {
    for (const file of [
      "{name:'payload.txt',buffer:bytes('payload')}",
      "at('path.txt')",
    ]) {
      const outcome = await run(
        '/replace.html',
        PRELUDE +
          `
        const result=await settle(page.locator('#file').setInputFiles(${file}));
        await page.waitForFunction(()=>events.includes('change'));
        return {result,events:await page.evaluate(()=>events),current:await page.locator('#file').evaluate(e=>e.files.length),old:await page.evaluate(()=>({connected:saved.isConnected,files:[...saved.files].map(f=>f.name)}))};
      `,
        '({events,current:document.querySelector("input").files.length})',
        { 'path.txt': 'path' },
      )
      assert.equal(outcome.playwright.error, undefined)
      assert.equal(outcome.neo.error, undefined)
      assert.deepEqual(outcome.neo.value, outcome.playwright.value)
      assert.deepEqual(outcome.neo.probe, outcome.playwright.probe)
      assert.equal(outcome.neo.value.result, 'resolved')
      assert.deepEqual(outcome.neo.value.events, ['input', 'change'])
      assert.equal(outcome.neo.value.current, 0)
      assert.equal(outcome.neo.value.old.connected, false)
      assert.equal(outcome.neo.value.old.files.length, 1)
    }
  })
})

const PRELUDE = `
  const bytes = text => typeof Buffer === 'function' ? Buffer.from(text) : Uint8Array.from(text, c => c.charCodeAt(0));
  const settle = promise => promise.then(() => 'resolved', error => error.name);
`

const CHILD = `<input id="file" type="file" disabled style="display:none"><label id="label" for="file">Upload</label>
  <div id="dropzone" style="width:150px;height:80px">Drop here</div>
  <script>
  window.events=[];
  for (const type of ['input','change']) document.addEventListener(type,e=>events.push({type,bubbles:e.bubbles}));
  window.facts=async()=>({events,files:await Promise.all([...document.querySelector('input').files].map(async f=>({name:f.name,type:f.type,text:await f.text()})))});
  dropzone.addEventListener('dragover',e=>e.preventDefault());
  dropzone.addEventListener('drop',e=>window.lastDrop=e.dataTransfer.getData('text/plain'));
  </script>`

test('a directory upload can name the artifact root with dot', {
  skip,
  timeout: 120_000,
}, async (t) => {
  await withEngines(
    t,
    { pages: { '/root.html': '<input type="file" webkitdirectory>' } },
    async (run) => {
      const result = await run(
        '/root.html',
        PRELUDE +
          `
      return await settle(page.locator('input').setInputFiles(at('.')));
    `,
        'Promise.all([...document.querySelector("input").files].map(async f=>({name:f.name,text:await f.text()})))',
        { 'one.txt': 'one' },
      )
      t.diagnostic(
        JSON.stringify({
          playwright: {
            value: result.playwright.value,
            probe: result.playwright.probe,
          },
          neo: {
            value: result.neo.value,
            probe: result.neo.probe,
            calls: result.neo.outcome.calls,
          },
        }),
      )
      assert.equal(result.playwright.value, 'resolved')
      assert.deepEqual(result.playwright.probe, [
        { name: 'one.txt', text: 'one' },
      ])
      assert.deepEqual(result.neo.value, result.playwright.value)
      assert.deepEqual(result.neo.probe, result.playwright.probe)
    },
  )
})

test('hidden inputs upload through labels in same-process and remote frames', {
  skip,
  timeout: 120_000,
}, async (t) => {
  const top = `<input type="file" id="decoy"><iframe name="local" src="/child.html" style="visibility:hidden"></iframe>
    <iframe name="remote" style="visibility:hidden"></iframe>
    <script>document.querySelector('[name=remote]').src='http://localhost:'+location.port+'/child.html'</script>`
  await withEngines(
    t,
    { pages: { '/top.html': top, '/child.html': CHILD } },
    async (run) => {
      const result = await run(
        '/top.html',
        PRELUDE +
          `
      await page.frameLocator('[name=remote]').locator('#file').waitFor({state:'attached'});
      await page.frame('local').setInputFiles('#label', {name:'local.json',buffer:bytes('{}')});
      await page.frameLocator('[name=remote]').locator('#label').setInputFiles(at('remote.txt'));
      await page.evaluate(()=>document.querySelector('[name=remote]').style.visibility='visible');
      await page.frameLocator('[name=remote]').locator('#dropzone').drop({data:{'text/plain':'remote-drop'}});
      return {local:await page.frame('local').evaluate(()=>facts()),remote:await page.frame('remote').evaluate(()=>facts()),drop:await page.frame('remote').evaluate(()=>window.lastDrop)};
    `,
        'document.querySelector("#decoy").files.length',
        { 'remote.txt': 'remote' },
      )
      assert.equal(result.playwright.error, undefined)
      assert.equal(result.neo.error, undefined)
      assert.deepEqual(result.neo.value, result.playwright.value)
      assert.deepEqual(result.neo.probe, 0)
      assert.deepEqual(result.playwright.probe, 0)
      assert.equal(result.neo.value.drop, 'remote-drop')
      for (const who of ['local', 'remote'])
        assert.deepEqual(result.neo.value[who].events, [
          { type: 'input', bubbles: true },
          { type: 'change', bubbles: true },
        ])
      assert.deepEqual(result.neo.value.local.files, [
        { name: 'local.json', type: 'application/json', text: '{}' },
      ])
      assert.deepEqual(result.neo.value.remote.files, [
        { name: 'remote.txt', type: 'text/plain', text: 'remote' },
      ])
    },
  )
})

test('refused path and over-bound payload leave the prior file list and events intact', {
  skip,
  timeout: 120_000,
}, async (t) => {
  const html = `<input id="file" type="file" multiple><script>
    window.events=[];for(const type of ['input','change']) document.addEventListener(type,e=>events.push(e.type));
    window.facts=()=>({events:[...events],files:[...document.querySelector('input').files].map(f=>({name:f.name,size:f.size}))});
    </script>`
  await withEngines(t, { pages: { '/upload.html': html } }, async (run) => {
    const result = await run(
      '/upload.html',
      PRELUDE +
        `
      await page.locator('#file').setInputFiles({name:'kept.txt',buffer:bytes('keep')});
      const path = await settle(page.locator('#file').setInputFiles([at('inside.txt'),at('../outside.txt')]));
      const afterPath = await page.evaluate(()=>facts());
      await page.locator('#file').setInputFiles({name:'kept.txt',buffer:bytes('keep')});
      const beforeBig = await page.evaluate(()=>facts());
      const big = new Uint8Array(7*1024*1024);
      const large = await settle(page.locator('#file').setInputFiles([
        {name:'small.txt',buffer:bytes('small')},
        {name:'large.bin',buffer:typeof Buffer === 'function' ? Buffer.from(big) : big},
      ]));
      return {path,afterPath,beforeBig,large,afterBig:await page.evaluate(()=>facts())};
    `,
      'facts()',
      { 'inside.txt': 'inside', '../outside.txt': 'outside' },
    )
    assert.equal(result.playwright.error, undefined)
    assert.equal(result.neo.error, undefined)
    assert.equal(result.playwright.value.path, 'resolved')
    assert.equal(result.playwright.value.large, 'resolved')
    assert.equal(result.neo.value.path, 'Error')
    assert.equal(result.neo.value.large, 'Error')
    assert.deepEqual(result.neo.value.afterPath, {
      events: ['input', 'change'],
      files: [{ name: 'kept.txt', size: 4 }],
    })
    assert.deepEqual(result.neo.value.afterBig, result.neo.value.beforeBig)
    assert.deepEqual(result.neo.probe, result.neo.value.afterBig)
    assert.deepEqual(result.playwright.value.afterBig.files, [
      { name: 'small.txt', size: 5 },
      { name: 'large.bin', size: 7 * 1024 * 1024 },
    ])
  })
})

test('upload and drop cancellation preserve the page and drop data reaches main-world listeners', {
  skip,
  timeout: 120_000,
}, async (t) => {
  const html = `<input id="file" type="file"><div id="zone" style="width:200px;height:100px"></div><script>
    window.events=[];document.querySelector('input').addEventListener('change',()=>events.push('change'));
    zone.addEventListener('dragover',e=>e.preventDefault());
    zone.addEventListener('drop',async e=>{events.push('drop');window.result={
      types:[...e.dataTransfer.types].sort(),text:e.dataTransfer.getData('Text'),
      html:e.dataTransfer.getData('text/html'),files:await Promise.all([...e.dataTransfer.files].map(async f=>({name:f.name,text:await f.text()})))};});
    </script>`
  await withEngines(t, { pages: { '/cancel.html': html } }, async (run) => {
    const result = await run(
      '/cancel.html',
      PRELUDE +
        `
      const outcomes=[];
      for(const action of ['upload','drop']) {
        const controller=new AbortController();controller.abort();
        const options={signal:controller.signal};
        outcomes.push(await settle(action==='upload'
          ? page.locator('#file').setInputFiles({name:'no.txt',buffer:bytes('no')},options)
          : page.locator('#zone').drop({data:{'text/plain':'no'}},options)));
      }
      const before=await page.evaluate(()=>[...events]);
      await page.locator('#zone').drop({files:{name:'real.txt',buffer:bytes('yes')},data:{Text:'plain','text/html':'<b>bold</b>'}});
      await page.waitForFunction(()=>window.result);
      return {outcomes,before,result:await page.evaluate(()=>result)};
    `,
      '({events,files:document.querySelector("input").files.length})',
    )
    assert.equal(result.playwright.error, undefined)
    assert.equal(result.neo.error, undefined)
    assert.deepEqual(result.neo.value, result.playwright.value)
    assert.deepEqual(result.neo.probe, result.playwright.probe)
    assert.deepEqual(result.neo.value.outcomes, ['AbortError', 'AbortError'])
    assert.deepEqual(result.neo.value.before, [])
    assert.deepEqual(result.neo.value.result, {
      types: ['Files', 'text/html', 'text/plain'],
      text: 'plain',
      html: '<b>bold</b>',
      files: [{ name: 'real.txt', text: 'yes' }],
    })
    assert.deepEqual(result.neo.probe, { events: ['drop'], files: 0 })
  })
})
