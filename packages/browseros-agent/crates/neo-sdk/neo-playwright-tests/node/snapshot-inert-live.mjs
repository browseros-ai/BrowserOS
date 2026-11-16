/** Production prelude + native SDK + real DOM: inertness changes what the caller can act on. */
import assert from 'node:assert/strict'
import http from 'node:http'
import { openClaw } from './claw-loader.mjs'

const fixture = `<!doctype html><main id="background"><p>Background prose</p><button>Background action</button></main>
<dialog id="modal"><p>Read this first</p><button>Modal action</button></dialog>
<script>
const shape = new URL(location.href).searchParams.get('shape');
const dlg = document.getElementById('modal');
if (shape === 'inert') background.inert = true;
if (shape === 'native') dlg.showModal();
if (shape === 'nested') { background.inert = true; background.append(dlg); dlg.showModal(); }
if (shape === 'shadow') { const host = document.createElement('div'); document.body.append(host); host.attachShadow({mode:'open'}).append(dlg); dlg.showModal(); }
if (shape === 'slotted-modal') {
  background.inert = true;
  const host = document.createElement('div'); background.append(host);
  const action = dlg.querySelector('button'); action.slot = 'action'; host.append(action);
  dlg.innerHTML = '<slot name="action"></slot>';
  host.attachShadow({mode:'open'}).append(dlg); dlg.showModal();
}
if (shape === 'aria-modal') { dlg.setAttribute('role','dialog'); dlg.setAttribute('aria-modal','true'); dlg.show(); }
if (shape === 'aria-hidden') background.setAttribute('aria-hidden','true');
if (shape === 'overlay') document.body.insertAdjacentHTML('beforeend','<div style="position:fixed;inset:0;background:white">Overlay</div>');
</script>`
const server = http.createServer((_req, res) => { res.setHeader('content-type', 'text/html'); res.end(fixture) })
await new Promise(resolve => server.listen(0, '127.0.0.1', resolve))
const sdk = await openClaw(Number(process.env.BROWSEROS_CDP_PORT), { timeoutMs: 120000 }, { upstreamSnapshots: false })
try {
  for (const shape of ['native', 'inert', 'nested', 'shadow', 'slotted-modal', 'aria-modal', 'aria-hidden', 'overlay']) {
    const page = await sdk.context.newPage()
    try {
      await page.goto(`http://127.0.0.1:${server.address().port}/?shape=${shape}`)
      const upstreamText = await page.ariaSnapshot({ mode: 'default' })
      const beforeJson = await page.ariaSnapshotJSON({ mode: 'ai' })
      const text = await page.ariaSnapshot()
      if (['native', 'inert', 'nested', 'slotted-modal'].includes(shape)) assert.match(text, /^# not listed:/, shape)
      const original = await page.ariaSnapshot({ inert: true })
      const full = await page.ariaSnapshot({ mode: 'full' })
      if (['native', 'inert', 'nested', 'slotted-modal'].includes(shape)) {
        assert.match(text, /^# not listed:/, shape)
        assert.doesNotMatch(text, /Background action/, shape)
        assert.match(original, /Background action/, shape)
        assert.match(full, /^# inert:/, shape)
        assert.match(full, /Background action/, shape)
        if (shape !== 'inert') assert.match(text, /Modal action.*ref=/, shape)
        for (const maxTokens of [1, 2, 10, 20, 100]) assert.match(await page.ariaSnapshot({ maxTokens }), /^# not listed:/, shape)
      } else {
        assert.match(text, /Background action/, shape)
        assert.doesNotMatch(text, /^# not listed:/, shape)
        if (shape === 'shadow') assert.match(text, /Modal action.*ref=/, shape)
      }
      // JSON is deliberately the unannotated tree even on the same world after a marked read.
      const json = await page.ariaSnapshotJSON({ mode: 'ai' })
      assert.doesNotMatch(JSON.stringify(json), /"inert":/)
      assert.deepEqual(json, beforeJson, `${shape}: JSON bytes unchanged by annotation`)
      assert.equal(await page.ariaSnapshot({ mode: 'default' }), upstreamText)
      console.log(JSON.stringify({ shape, text, original, full }))
    } finally { await page.close() }
  }
} finally {
  await sdk.finish()
  await new Promise(resolve => server.close(resolve))
}
