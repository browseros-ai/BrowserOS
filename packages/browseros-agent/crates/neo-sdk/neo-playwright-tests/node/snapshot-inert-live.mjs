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
if (shape === 'inert-image') {
  background.inert = true;
  background.innerHTML = '<a href="#article"><img alt="Article image" style="pointer-events:none;width:80px;height:30px" src="data:image/svg+xml,%3Csvg xmlns=%22http://www.w3.org/2000/svg%22 width=%2280%22 height=%2230%22/%3E"></a>';
}
if (shape === 'native') dlg.showModal();
if (shape === 'modal-inert-section') {
  dlg.innerHTML = '<section inert><button id="blocked">Blocked modal action</button></section><button id="live">Live modal action</button>';
  dlg.showModal();
}
if (shape === 'modal-inert-self') { dlg.inert = true; dlg.showModal(); }
if (shape === 'modal-inert-nested') {
  dlg.innerHTML = '<section inert><button id="blocked">Blocked modal action</button><dialog id="inner"><button id="live">Inner modal action</button></dialog></section>';
  dlg.showModal(); document.getElementById('inner').showModal();
}
if (shape === 'stacked') {
  const lower = dlg.cloneNode(true); lower.id = 'lower';
  const button = lower.querySelector('button'); button.textContent = 'Lower modal action';
  document.body.append(lower); lower.showModal(); dlg.showModal();
  button.focus(); document.body.dataset.lowerModalFocused = String(document.activeElement === button);
}
if (shape === 'nested') { background.inert = true; background.append(dlg); dlg.showModal(); }
if (shape === 'shadow') { const host = document.createElement('div'); document.body.append(host); host.attachShadow({mode:'open'}).append(dlg); dlg.showModal(); }
if (shape === 'slotted-modal') {
  background.inert = true;
  const host = document.createElement('div'); background.append(host);
  const action = dlg.querySelector('button'); action.slot = 'action'; host.append(action);
  dlg.innerHTML = '<slot name="action"></slot>';
  host.attachShadow({mode:'open'}).append(dlg); dlg.showModal();
}
if (shape === 'shadow-no-refs') {
  background.inert = true;
  dlg.style.pointerEvents = 'none';
  background.attachShadow({mode:'open'}).append(dlg); dlg.showModal();
}
if (shape === 'shadow-wrapped-no-refs') {
  background.inert = true; dlg.style.pointerEvents = 'none';
  const wrap = document.createElement('section'); wrap.setAttribute('aria-label', 'Wrapper');
  background.attachShadow({mode:'open'}).append(wrap); wrap.append(dlg); dlg.showModal();
}
if (shape === 'shadow-modal-beside-inert') {
  const side = document.createElement('aside'); side.inert = true;
  side.innerHTML = '<button>Side action</button>'; document.body.append(side);
  const host = document.createElement('div'); document.body.append(host);
  host.attachShadow({mode:'open'}).append(dlg); dlg.showModal();
}
if (shape === 'closed-modal' || shape === 'closed-modal-no-refs') {
  background.inert = true;
  if (shape === 'closed-modal-no-refs') dlg.style.pointerEvents = 'none';
  const host = document.createElement('div'); background.append(host);
  host.attachShadow({mode:'closed'}).append(dlg); dlg.showModal();
}
if (shape === 'slotted-inert') {
  const host = document.createElement('div'); document.body.append(host);
  const action = background.querySelector('button'); action.slot = 'action'; host.append(action);
  host.attachShadow({mode:'open'}).innerHTML = '<div inert><slot name="action"></slot></div>';
}
if (shape === 'closed-inert') {
  background.attachShadow({mode:'closed'}).innerHTML = '<div inert><button>Hidden shadow action</button></div>';
}
if (shape === 'shadow-flattened-modal') {
  background.inert = true;
  background.attachShadow({mode:'open'}).innerHTML = '<dialog role="none">Live modal message</dialog>';
  background.shadowRoot.querySelector('dialog').showModal();
}
if (shape === 'delta') {
  background.querySelector('button').onclick = () => dlg.showModal();
  dlg.querySelector('button').onclick = () => dlg.close();
}
if (shape === 'aria-modal') { dlg.setAttribute('role','dialog'); dlg.setAttribute('aria-modal','true'); dlg.show(); }
if (shape === 'aria-hidden') background.setAttribute('aria-hidden','true');
if (shape === 'overlay') document.body.insertAdjacentHTML('beforeend','<div style="position:fixed;inset:0;background:white">Overlay</div>');
// Ask the browser directly, before the snapshot: SDK actionability must not decide this row.
if (shape.startsWith('modal-inert-') || ['nested', 'slotted-modal', 'slotted-inert'].includes(shape)) {
  const focus = {};
  for (const button of document.querySelectorAll('button')) {
    button.focus(); focus[button.textContent] = document.activeElement === button;
  }
  document.body.dataset.focusVerdicts = JSON.stringify(focus);
}
</script>`
const server = http.createServer((_req, res) => {
  res.setHeader('content-type', 'text/html')
  res.end(fixture)
})
await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve))
const sdk = await openClaw(
  Number(process.env.BROWSEROS_CDP_PORT),
  { timeoutMs: 120000 },
  { upstreamSnapshots: false },
)
try {
  for (const shape of [
    'native',
    'modal-inert-section',
    'modal-inert-self',
    'modal-inert-nested',
    'stacked',
    'inert',
    'inert-image',
    'nested',
    'shadow',
    'slotted-modal',
    'slotted-inert',
    'closed-inert',
    'shadow-no-refs',
    'shadow-wrapped-no-refs',
    'shadow-modal-beside-inert',
    'closed-modal',
    'closed-modal-no-refs',
    'shadow-flattened-modal',
    'aria-modal',
    'aria-hidden',
    'overlay',
  ]) {
    const page = await sdk.context.newPage()
    try {
      await page.goto(
        `http://127.0.0.1:${server.address().port}/?shape=${shape}`,
      )
      // Closed-root opening belongs to marked snapshots independently of inertness.
      // Establish that existing view before comparing later unannotated JSON/YAML reads.
      if (
        ['closed-inert', 'closed-modal', 'closed-modal-no-refs'].includes(shape)
      )
        await page.ariaSnapshot({ inert: true })
      const upstreamText = await page.ariaSnapshot({ mode: 'default' })
      const beforeJson = await page.ariaSnapshotJSON({ mode: 'ai' })
      const text = await page.ariaSnapshot()
      const omitted = [
        'native',
        'stacked',
        'inert',
        'nested',
        'slotted-modal',
        'slotted-inert',
        'closed-modal',
      ].includes(shape)
      const original = await page.ariaSnapshot({ inert: true })
      const full = await page.ariaSnapshot({ mode: 'full' })
      if (['nested', 'slotted-modal', 'slotted-inert'].includes(shape)) {
        const focus = JSON.parse(
          await page.locator('body').getAttribute('data-focus-verdicts'),
        )
        assert.equal(focus['Background action'], false, shape)
        assert.equal(focus['Modal action'], shape !== 'slotted-inert', shape)
        console.log(JSON.stringify({ shape, focus }))
      }
      if (shape.startsWith('modal-inert-')) {
        const focus = JSON.parse(
          await page.locator('body').getAttribute('data-focus-verdicts'),
        )
        assert.equal(focus['Background action'], false, shape)
        if (shape === 'modal-inert-self') {
          assert.equal(focus['Modal action'], false, shape)
          assert.doesNotMatch(text, /Modal action/, shape)
        } else {
          assert.equal(focus['Blocked modal action'], false, shape)
          const live =
            shape === 'modal-inert-section'
              ? 'Live modal action'
              : 'Inner modal action'
          assert.equal(focus[live], true, shape)
          assert.match(text, new RegExp(`${live}.*ref=`), shape)
          assert.doesNotMatch(text, /Blocked modal action/, shape)
        }
        assert.match(
          text,
          /^# not listed: \d+ lines \([1-9]\d* with refs\)/,
          shape,
        )
        assert.doesNotMatch(text, /Background action/, shape)
        console.log(JSON.stringify({ shape, focus, text }))
      }
      if (shape === 'stacked') {
        // Chromium refuses focus in the lower modal. PR1 deliberately lists either modal:
        // it has no top-layer ordering test, so the conservative rule retains this control.
        assert.equal(
          await page.locator('body').getAttribute('data-lower-modal-focused'),
          'false',
        )
        assert.match(text, /Lower modal action.*ref=/)
      }
      if (omitted) {
        assert.match(text, /^# not listed:/, shape)
        assert.doesNotMatch(text, /Background action/, shape)
        assert.match(original, /Background action/, shape)
        assert.match(full, /^# inert:/, shape)
        assert.match(full, /Background action/, shape)
        if (!['inert', 'slotted-inert'].includes(shape))
          assert.match(text, /Modal action.*ref=/, shape)
        for (const maxTokens of [1, 2, 10, 20, 100])
          assert.match(
            await page.ariaSnapshot({ maxTokens }),
            /^# not listed:/,
            shape,
          )
      } else if (shape.startsWith('modal-inert-')) {
        // Browser focus verdicts, omission counts and live refs were checked above.
        assert.match(full, /^# inert:/, shape)
        assert.match(original, /Background action/, shape)
      } else if (shape === 'inert-image') {
        assert.match(text, /^# not listed:/, shape)
        assert.doesNotMatch(text, /Article image|link/, shape)
        assert.match(original, /link.*ref=/, shape)
      } else if (shape === 'shadow-modal-beside-inert') {
        assert.match(text, /^# not listed:/, shape)
        assert.match(text, /Background action/, shape)
        assert.match(text, /Modal action.*ref=/, shape)
        assert.doesNotMatch(text, /Side action/, shape)
        assert.match(original, /Side action/, shape)
      } else if (shape === 'closed-modal-no-refs') {
        assert.match(text, /^# not listed:/, shape)
        assert.match(text, /Read this first.*Modal action/s, shape)
        assert.doesNotMatch(text, /Background action/, shape)
        assert.match(original, /Background action/, shape)
      } else if (shape === 'closed-inert') {
        assert.match(text, /^# not listed:/, shape)
        assert.doesNotMatch(text, /Hidden shadow action/, shape)
        assert.match(original, /Hidden shadow action/, shape)
      } else if (shape === 'shadow-flattened-modal') {
        assert.match(text, /Live modal message/, shape)
        assert.doesNotMatch(text, /^# not listed:/, shape)
      } else if (['shadow-no-refs', 'shadow-wrapped-no-refs'].includes(shape)) {
        assert.match(text, /Read this first.*Modal action/s, shape)
        assert.doesNotMatch(text, /^# not listed:/, shape)
      } else {
        assert.match(text, /Background action/, shape)
        assert.doesNotMatch(text, /^# not listed:/, shape)
        if (shape === 'shadow') assert.match(text, /Modal action.*ref=/, shape)
      }
      // JSON is deliberately the unannotated tree even on the same world after a marked read.
      const json = await page.ariaSnapshotJSON({ mode: 'ai' })
      assert.doesNotMatch(JSON.stringify(json), /"inert":/)
      assert.deepEqual(
        json,
        beforeJson,
        `${shape}: JSON bytes unchanged by annotation`,
      )
      assert.equal(await page.ariaSnapshot({ mode: 'default' }), upstreamText)
      console.log(JSON.stringify({ shape, text, original, full }))
    } finally {
      await page.close()
    }
  }
  const page = await sdk.context.newPage()
  try {
    // The external-JS harness returns no script value, so it commits no cross-run baseline.
    // Exercise its uncompared delta reads here; snapshot-inert.test.ts pins the comparison.
    await page.goto(`http://127.0.0.1:${server.address().port}/?shape=delta`)
    const before = await page.ariaSnapshot()
    await page.getByRole('button', { name: 'Background action' }).click()
    const opening = await page.ariaSnapshot({ delta: true })
    assert.match(opening, /^# whole page: no earlier snapshot of this page/)
    assert.match(opening, /# not listed:/)
    assert.doesNotMatch(opening, /Background action/)
    await page.getByRole('button', { name: 'Modal action' }).click()
    const closing = await page.ariaSnapshot({ delta: true })
    assert.match(closing, /Background action/)
    assert.match(before, /Background action/)
    console.log(JSON.stringify({ shape: 'delta', before, opening, closing }))
  } finally {
    await page.close()
  }
  await sdk.finish()
  const conformance = await openClaw(Number(process.env.BROWSEROS_CDP_PORT), {
    timeoutMs: 120000,
  })
  try {
    for (const shape of ['native', 'inert']) {
      const page = await conformance.context.newPage()
      try {
        await page.goto(
          `http://127.0.0.1:${server.address().port}/?shape=${shape}`,
        )
        const before = await page.ariaSnapshot()
        assert.equal(await page.ariaSnapshot({ inert: true }), before)
        assert.doesNotMatch(before, /^# (?:not listed|inert):/)
        console.log(
          JSON.stringify({ shape: `conformance-${shape}`, text: before }),
        )
      } finally {
        await page.close()
      }
    }
  } finally {
    await conformance.finish()
  }
} finally {
  await sdk.finish()
  await new Promise((resolve) => server.close(resolve))
}
