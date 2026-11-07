import assert from 'node:assert/strict'
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { test } from 'node:test'
import {
  browserBinary,
  buildNeoRun,
  chromium,
  compareMethod,
  launchBrowser,
  runCleanups,
  serveFixture,
  skip,
  startNeo,
} from './harness.mjs'

const removeTakeover = `<script>new MutationObserver(() => document.querySelector('browseros-neo-takeover')?.remove()).observe(document.documentElement,{childList:true,subtree:true})</script>`

test('dispatchEvent accepts one plain touch reused in its three touch lists', {
  skip,
  timeout: 120_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html: `<button id="target">Touch target</button><output id="seen">[]</output><script>
      target.addEventListener('touchstart', e => {
        seen.textContent=JSON.stringify([e.touches,e.targetTouches,e.changedTouches].map(list=>Array.from(list,t=>({id:t.identifier,x:t.clientX,y:t.clientY,target:t.target.id}))));
      });</script>${removeTakeover}`,
    source: `const point={identifier:7,clientX:13,clientY:17}; let result;
      try {await page.locator('#target').dispatchEvent('touchstart',{touches:[point],targetTouches:[point],changedTouches:[point]});result='resolved'}
      catch(error){result=error.name}
      let control; try {await page.locator('#target').dispatchEvent('touchstart',{touches:[{...point}],targetTouches:[{...point}],changedTouches:[{...point}]});control='resolved'} catch(error){control=error.name}
      return {result,control,seen:await page.locator('#seen').textContent()};`,
    probe: `document.querySelector('#seen').textContent`,
  })
  assert.equal(outcome.value.result, 'resolved')
  assert.deepEqual(
    JSON.parse(outcome.value.seen),
    Array.from({ length: 3 }, () => [
      { id: 7, x: 13, y: 17, target: 'target' },
    ]),
  )
})

test('highlight cleanup reaches a frame again after its process changes', {
  skip,
  timeout: 180_000,
}, async (t) => {
  const executable = buildNeoRun(),
    cleanups = [],
    results = []
  try {
    const origin = await serveFixture(cleanups, {
      pages: {
        '/main': `<iframe id="child" name="child" src="/old"></iframe><button id="swap" onclick="document.querySelector('#child').src='http://localhost:'+location.port+'/new'">Swap</button>${removeTakeover}`,
        '/old': `<button id="old">Old</button>${removeTakeover}`,
        '/new': `<button id="new">New</button>${removeTakeover}`,
      },
    })
    const browser = await launchBrowser(cleanups, browserBinary)
    const pw = await chromium.connectOverCDP(`http://127.0.0.1:${browser.port}`)
    cleanups.push(() => pw.close())
    const [context] = pw.contexts()
    const dir = mkdtempSync(join(tmpdir(), 'neo-b2-highlight-'))
    cleanups.push(() => rmSync(dir, { recursive: true, force: true }))
    for (const mode of ['run-end', 'hide-all']) {
      const page = await context.newPage()
      try {
        await page.goto(`${origin}/main`)
        await page.frameLocator('#child').locator('#old').waitFor()
        const session = await context.newCDPSession(page)
        const { targetInfo } = await session.send('Target.getTargetInfo')
        const before = await session.send('Page.getFrameTree')
        const script = join(dir, mode + '.js')
        writeFileSync(
          script,
          `await page.frameLocator('#child').locator('#old').highlight();
          await page.locator('#swap').click();
          await page.frameLocator('#child').locator('#new').waitFor();
          await page.frameLocator('#child').locator('#new').highlight();
          ${mode === 'hide-all' ? 'await page.hideHighlight();' : ''}
          return 'done';`,
        )
        const outcome = await startNeo(
          executable,
          browser.port,
          targetInfo.targetId,
          script,
        ).outcome
        const after = await session.send('Page.getFrameTree')
        const targets = await session.send('Target.getTargets')
        const overlays = await page
          .frameLocator('#child')
          .locator('x-pw-glass')
          .count()
        const remote = targets.targetInfos.find(
          (x) => x.type === 'iframe' && x.url.endsWith('/new'),
        )
        results.push({
          mode,
          status: outcome.status,
          overlays,
          oldFrame: before.frameTree.childFrames?.[0]?.frame.id,
          newFrame:
            after.frameTree.childFrames?.[0]?.frame.id ?? remote?.targetId,
          remote: !!remote,
        })
        await session.detach()
      } finally {
        await page.close()
      }
    }
    t.diagnostic(JSON.stringify(results))
    for (const r of results) {
      assert.equal(r.status, 'ok', JSON.stringify(r))
      assert.equal(r.remote, true, JSON.stringify(r))
      assert.equal(
        r.oldFrame,
        r.newFrame,
        'same iframe identity across process swap',
      )
      assert.equal(
        r.overlays,
        0,
        `${r.mode} must remove the new document's highlight`,
      )
    }
  } finally {
    await runCleanups(cleanups)
  }
})

test('element waits release the input turn and the B2 workflow completes', {
  skip,
  timeout: 180_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html: `<button id="insert" onclick="const e=document.createElement('input');e.id='late';document.body.append(e)">Insert</button>
      <button id="reveal" onclick="document.querySelector('#hidden').style.display='block'">Reveal</button>
      <button id="add-blur" onclick="const e=document.createElement('input');e.id='blur-late';document.body.append(e)">Add</button>
      <div id="hidden" contenteditable style="display:none">Selected text</div><output id="seen">none</output>
      <script>document.addEventListener('ping',()=>seen.textContent='pinged')</script>${removeTakeover}`,
    source: `const focusing=page.locator('#late').focus(); await page.locator('#insert').click(); await focusing;
      const selecting=page.locator('#hidden').selectText(); await page.locator('#reveal').click(); await selecting;
      const blurring=page.locator('#blur-late').blur(); await page.locator('#add-blur').click(); await blurring;
      const target=page.locator('#late'); await target.scrollIntoViewIfNeeded(); const box=await target.boundingBox();
      await target.dispatchEvent('ping'); const highlight=await target.highlight(); await highlight.dispose(); await page.hideHighlight();
      return {box:!!box&&box.width>0&&box.height>0,event:await page.locator('#seen').textContent()};`,
    probe: `({selection:window.getSelection().toString(),overlays:document.querySelectorAll('x-pw-glass').length})`,
  })
  assert.deepEqual(outcome.value, { box: true, event: 'pinged' })
})
