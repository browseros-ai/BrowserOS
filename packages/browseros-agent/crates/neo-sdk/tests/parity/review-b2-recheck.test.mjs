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

const noTakeover = `<script>new MutationObserver(()=>document.querySelector('browseros-neo-takeover')?.remove()).observe(document.documentElement,{childList:true,subtree:true})</script>`

test('review: a shared graph expanding past 100k values fails before dispatch', {
  skip,
  timeout: 120_000,
}, async (t) => {
  const outcome = await compareMethod(t, {
    html: `<button id="target">Target</button><output id="seen">0</output><script>target.addEventListener('ping',()=>seen.textContent=String(+seen.textContent+1))</script>${noTakeover}`,
    source: `const shared={leaf:true};const value=Array(60_000).fill(shared);
      let large;try{await page.locator('#target').dispatchEvent('ping',{detail:value});large='resolved'}catch(error){large=error.name}
      const before=await page.locator('#seen').textContent();
      await page.locator('#target').dispatchEvent('ping',{});
      return {large,before,after:await page.locator('#seen').textContent()};`,
    probe: `document.querySelector('#seen').textContent`,
    values: ({ playwright, neo }) => {
      assert.deepEqual(playwright, {
        large: 'resolved',
        before: '1',
        after: '2',
      })
      assert.deepEqual(neo, { large: 'Error', before: '0', after: '1' })
    },
    effects: ({ playwright, neo }) => {
      assert.equal(playwright, '2')
      assert.equal(neo, '1')
    },
  })
  t.diagnostic(JSON.stringify(outcome.value))
})

// Only the chosen glue callback is delayed; its original implementation, all other
// calls, and the renderer remain live. This is controlled ordering, not normal timing.
async function delayedWrite(
  t,
  { source, target = 'B', method = 'setHighlights', observe },
) {
  const cleanups = []
  try {
    const origin = await serveFixture(cleanups, {
      pages: {
        '/main': `<iframe name="A" src="/a"></iframe><iframe name="B" id="b"></iframe>
        <button id="arm" onclick="document.body.insertAdjacentHTML('beforeend','<i id=armed></i>')">Arm</button>
        <button id="release" onclick="window.postMessage('release-highlight','*')">Release</button>
        <button id="mark" onclick="document.body.insertAdjacentHTML('beforeend','<i id=ready></i>')">Mark</button>
        <button id="mark2" onclick="document.body.insertAdjacentHTML('beforeend','<i id=ready2></i>')">Mark again</button>
        <script>b.src='http://localhost:'+location.port+'/b';addEventListener('message',e=>{if(e.data==='highlight-pending')document.body.insertAdjacentHTML('beforeend','<i id=blocked></i>')})</script>${noTakeover}`,
        '/a': `<button id="old">Old A</button><button id="new">New A</button>${noTakeover}`,
        '/b': `<button id="old">Old B</button><button id="new">New B</button><button id="release" onclick="window.postMessage('release-highlight','*');document.querySelector('#clicked').textContent='yes'">Release</button><output id="clicked">no</output>${noTakeover}`,
      },
    })
    const browser = await launchBrowser(cleanups, browserBinary)
    const pw = await chromium.connectOverCDP(`http://127.0.0.1:${browser.port}`)
    cleanups.push(() => pw.close())
    const [context] = pw.contexts(),
      page = await context.newPage()
    await page.goto(origin + '/main')
    const session = await context.newCDPSession(page)
    const { targetInfo } = await session.send('Target.getTargetInfo')
    const dir = mkdtempSync(join(tmpdir(), 'neo-b2r-'))
    cleanups.push(() => rmSync(dir, { recursive: true, force: true }))
    const path = join(dir, 'script.js')
    writeFileSync(path, source)
    const run = startNeo(
      buildNeoRun(),
      browser.port,
      targetInfo.targetId,
      path,
    ).outcome
    await page.locator('#armed').waitFor({ state: 'attached', timeout: 15_000 })
    const patchedSession =
      target === 'main'
        ? session
        : await context.newCDPSession(page.frame(target))
    const { frameTree } = await patchedSession.send('Page.getFrameTree')
    const world = await patchedSession.send('Page.createIsolatedWorld', {
      frameId: frameTree.frame.id,
      worldName: '__neo_utility_world__',
      grantUniveralAccess: true,
    })
    const patched = await patchedSession.send('Runtime.evaluate', {
      contextId: world.executionContextId,
      expression: `(()=>{const g=Object.values(globalThis.__neo_injected__??{})[0];if(!g||typeof g[${JSON.stringify(method)}]!=='function')return false;const old=g[${JSON.stringify(method)}];let once=false;g[${JSON.stringify(method)}]=async function(...args){if(!once){once=true;parent.postMessage('highlight-pending','*');await new Promise(resolve=>{const listen=e=>{if(e.data==='release-highlight'){removeEventListener('message',listen);resolve()}};addEventListener('message',listen)})}return old(...args)};return true})()`,
      returnByValue: true,
    })
    assert.equal(
      patched.result.value,
      true,
      'the real Neo glue exists before interception',
    )
    const marker = (id) =>
      page.evaluate(
        (id) =>
          document.body.insertAdjacentHTML('beforeend', `<i id="${id}"></i>`),
        id,
      )
    await marker('start')
    const observed = observe
      ? await observe({ page, context, marker })
      : undefined
    const outcome = await run
    assert.equal(outcome.status, 'ok', JSON.stringify(outcome))
    t.diagnostic(JSON.stringify({ value: outcome.value, observed }))
    return { outcome, observed }
  } finally {
    await runCleanups(cleanups)
  }
}

test('review: a pending highlight write does not block a click in the same frame', {
  skip,
  timeout: 120_000,
}, async (t) => {
  const { outcome } = await delayedWrite(t, {
    source: `const b=page.frame('B');await b.locator('#old').highlight();
    await page.locator('#arm').click();await page.locator('#start').waitFor({state:'attached'});
    const writing=b.locator('#new').highlight();await page.locator('#blocked').waitFor({state:'attached'});
    await b.locator('#release').click();await writing;return await b.locator('#clicked').textContent();`,
  })
  assert.equal(outcome.value, 'yes')
})

const walk = (n) => [
  n,
  ...(n.children ?? []).flatMap(walk),
  ...(n.shadowRoots ?? []).flatMap(walk),
]
test('review: an older hide-all keeps a newer highlight in another document', {
  skip,
  timeout: 120_000,
}, async (t) => {
  const { observed } = await delayedWrite(t, {
    target: 'main',
    method: 'hideHighlight',
    source: `const b=page.frame('B');await b.locator('#old').highlight();await page.locator('#arm').click();
      await page.locator('#start').waitFor({state:'attached'});const hiding=page.hideHighlight();
      await page.locator('#blocked').waitFor({state:'attached'});await b.locator('#new').highlight();
      await page.locator('#release').click();await hiding;
      await page.locator('#mark').click();await page.locator('#refresh').waitFor({state:'attached'});
      await b.locator('#new').highlight();await page.locator('#mark2').click();
      await page.locator('#hold').waitFor({state:'attached'});return 'done';`,
    observe: async ({ page, context, marker }) => {
      // Observe after hide-all has returned, before run-end cleanup.
      await page
        .locator('#ready')
        .waitFor({ state: 'attached', timeout: 15_000 })
      const b = await context.newCDPSession(page.frame('B'))
      const count = async () =>
        walk(
          (await b.send('DOM.getDocument', { depth: -1, pierce: true })).root,
        ).filter((n) => n.nodeName === 'X-PW-HIGHLIGHT').length
      const afterOldHide = await count()
      await marker('refresh')
      await page
        .locator('#ready2')
        .waitFor({ state: 'attached', timeout: 15_000 })
      const afterRefresh = await count()
      await marker('hold')
      return { afterOldHide, afterRefresh }
    },
  })
  assert.equal(
    observed.afterRefresh,
    1,
    'positive control sees the new highlight',
  )
  assert.equal(
    observed.afterOldHide,
    1,
    'an old hide must not erase a newer registry entry',
  )
})
