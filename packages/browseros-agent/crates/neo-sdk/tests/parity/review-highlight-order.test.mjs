// A delayed page callback models an in-flight highlight write. The underlying renderer and
// actual highlight code remain live, so newer work in another document can complete first.
import assert from 'node:assert/strict'
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { test } from 'node:test'
import {
  browserBinary,
  buildNeoRun,
  chromium,
  launchBrowser,
  runCleanups,
  serveFixture,
  skip,
  startNeo,
} from './harness.mjs'

const walk = (n) => [
  n,
  ...(n.children ?? []).flatMap(walk),
  ...(n.shadowRoots ?? []).flatMap(walk),
  ...(n.contentDocument ? walk(n.contentDocument) : []),
]
const hasId = (n, id) =>
  (n.attributes ?? []).some(
    (v, i, a) => i % 2 === 0 && v === 'id' && a[i + 1] === id,
  )

test('an in-flight old highlight write preserves the newer highlight in another document', {
  skip,
  timeout: 120_000,
}, async (t) => {
  const exe = buildNeoRun(),
    cleanups = []
  let result
  try {
    const origin = await serveFixture(cleanups, {
      pages: {
        '/main': `<iframe id="chosen" name="A" src="/a"></iframe><iframe id="spare" name="B"></iframe>
    <button id="arm" onclick="document.body.append(Object.assign(document.createElement('p'),{id:'armed'}))">Arm</button>
    <button id="switch" onclick="document.querySelector('#chosen').id='former';document.querySelector('#spare').id='chosen'">Switch</button>
    <button id="release" onclick="document.querySelector('#chosen').contentWindow.postMessage('release-highlight','*')">Release</button>
    <button id="mark" onclick="document.body.append(Object.assign(document.createElement('p'),{id:'ready'}))">Ready</button>
    <button id="mark2" onclick="document.body.append(Object.assign(document.createElement('p'),{id:'ready2'}))">Ready2</button>
    <script>document.querySelector('#spare').src='http://localhost:'+location.port+'/b';addEventListener('message',e=>{if(e.data==='highlight-pending')document.body.append(Object.assign(document.createElement('p'),{id:'blocked'}))})</script>`,
        '/a': '<button id="target">A target</button><button id="other">Newer highlight</button>',
        '/b': '<button id="target">B target</button>',
      },
    })
    const browser = await launchBrowser(cleanups, browserBinary)
    const pw = await chromium.connectOverCDP(`http://127.0.0.1:${browser.port}`)
    cleanups.push(() => pw.close())
    const [context] = pw.contexts()
    const page = await context.newPage()
    await page.goto(origin + '/main')
    const session = await context.newCDPSession(page)
    const { targetInfo } = await session.send('Target.getTargetInfo')
    const dir = mkdtempSync(join(tmpdir(), 'neo-b2-order-'))
    cleanups.push(() => rmSync(dir, { recursive: true, force: true }))
    const script = join(dir, 'race.js')
    writeFileSync(
      script,
      `const chosen=page.frameLocator('#chosen').locator('#target');await chosen.highlight();
   await page.frame('B').locator('#target').count();await page.locator('#arm').click();await page.locator('#start').waitFor({state:'attached'});
   await page.locator('#switch').click();const moving=chosen.highlight();await page.locator('#blocked').waitFor({state:'attached'});
   await page.frame('A').locator('#other').highlight();await page.locator('#release').click();await moving;
   await page.locator('#mark').click();await page.locator('#refresh').waitFor({state:'attached'});
   await page.frame('A').locator('#other').highlight();await page.locator('#mark2').click();await page.locator('#hold').waitFor({state:'attached'});return 'done';`,
    )
    const run = startNeo(exe, browser.port, targetInfo.targetId, script).outcome
    await page.locator('#armed').waitFor({ state: 'attached', timeout: 15000 })
    const bsession = await context.newCDPSession(page.frame('B'))
    const tree = await bsession.send('Page.getFrameTree')
    const world = await bsession.send('Page.createIsolatedWorld', {
      frameId: tree.frameTree.frame.id,
      worldName: '__neo_utility_world__',
      grantUniveralAccess: true,
    })
    const patched = await bsession.send('Runtime.evaluate', {
      contextId: world.executionContextId,
      expression: `(()=>{const g=Object.values(globalThis.__neo_injected__??{})[0];if(!g)return false;const old=g.setHighlights;let once=false;g.setHighlights=async function(...args){if(!once){once=true;parent.postMessage('highlight-pending','*');await new Promise(resolve=>{const listen=e=>{if(e.data==='release-highlight'){removeEventListener('message',listen);resolve()}};addEventListener('message',listen)});}return old(...args)};return true})()`,
      returnByValue: true,
    })
    assert.equal(
      patched.result.value,
      true,
      'B already has the actual Neo glue',
    )
    const marker = (id) =>
      page.evaluate(
        (id) =>
          document.body.append(
            Object.assign(document.createElement('p'), { id }),
          ),
        id,
      )
    await marker('start')
    await page.locator('#ready').waitFor({ state: 'attached', timeout: 15000 })
    const count = async () => {
      const { root } = await session.send('DOM.getDocument', {
        depth: -1,
        pierce: true,
      })
      const a = walk(root).find(
        (n) => n.nodeName === 'IFRAME' && hasId(n, 'former'),
      )
      assert.ok(a?.contentDocument)
      return walk(a.contentDocument).filter(
        (n) => n.nodeName === 'X-PW-HIGHLIGHT',
      ).length
    }
    const afterOldWrite = await count()
    await marker('refresh')
    await page.locator('#ready2').waitFor({ state: 'attached', timeout: 15000 })
    const afterRefresh = await count()
    await marker('hold')
    const outcome = await run
    result = { afterOldWrite, afterRefresh, status: outcome.status }
    t.diagnostic(JSON.stringify(result))
    assert.equal(outcome.status, 'ok', JSON.stringify(outcome))
    assert.equal(afterRefresh, 1, 'positive control sees A highlight')
    assert.equal(
      afterOldWrite,
      1,
      'an older saved list must not clear the newer A highlight',
    )
  } finally {
    await runCleanups(cleanups)
  }
})
