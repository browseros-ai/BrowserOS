// Neo: Forced frame clicks deliver through motion; ordinary clicks retain target stability checks.
import assert from 'node:assert/strict'
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { test } from 'node:test'
import {
  browserBinary,
  buildNeoRun,
  launchReference,
  playwrightVersion,
  runCleanups,
  runInNeo,
  runInPlaywright,
  serveFixture,
  skip,
} from './harness.mjs'

/** Neo: Each selector addresses a different host path through the same visible controls. */
const BUTTONS = `[
  page.frameLocator('#same').locator('#go'),
  page.frameLocator('#cross').locator('#go'),
  page.frameLocator('#cross').frameLocator('#nested').locator('#go')
]`

/** Neo: Public results and actual click counts distinguish waiting from forced delivery. */
const MOVING = `
  const results = [];
  for (const button of ${BUTTONS}) {
    let ordinary = 'unchecked';
    if (checkOrdinary) {
      ordinary = 'clicked';
      try { await button.click({ timeout: 300 }); }
      catch (error) { ordinary = error.name; }
    }
    const before = await button.getAttribute('data-clicks');
    let forced = 'clicked';
    try { await button.click({ force: true, timeout: 2000 }); }
    catch (error) { forced = error.name; }
    results.push({ ordinary, before, forced, after: await button.getAttribute('data-clicks') });
  }
  return results;
`

/** Neo: The same ordinary clicks must become actionable once the independent driver stops motion. */
const STOPPED = `
  const results = [];
  for (const button of ${BUTTONS}) {
    await button.click({ timeout: 5000 });
    results.push(await button.getAttribute('data-clicks'));
  }
  return results;
`

function document(moving, kind) {
  const frames =
    kind === 'top'
      ? '<iframe id=same src="/force-child.html?who=same"></iframe><iframe id=cross></iframe>'
      : kind === 'child'
        ? '<iframe id=nested></iframe>'
        : ''
  return `<!doctype html>
    <style>
      body { margin: 15px; }
      button { display: block; width: 160px; height: 40px; margin-bottom: 15px; }
      iframe { width: 300px; height: 330px; border: 7px solid; }
      #nested { width: 250px; height: 150px; }
    </style>
    <button id=go data-clicks=0 onclick="this.dataset.clicks=String(+this.dataset.clicks+1)">Go</button>
    ${frames}
    <script>
      ${kind === 'top' ? "document.querySelector('#cross').src='http://localhost:'+location.port+'/force-child.html?who=cross';" : ''}
      ${kind === 'child' ? "document.querySelector('#nested').src='http://127.0.0.1:'+location.port+'/force-leaf.html?who='+new URLSearchParams(location.search).get('who')+'-nested';" : ''}
      let moving = true, tick = 0;
      function animate() {
        if (!moving) return;
        const nodes = ${moving === 'owner' ? (kind === 'top' ? "document.querySelectorAll('iframe')" : '[]') : kind === 'top' ? '[]' : "[document.querySelector('#go')]"};
        for (const node of nodes) node.style.transform='translateX('+((tick%2)*8)+'px)';
        tick++;
        requestAnimationFrame(animate);
      }
      animate();
      window.stopMotion = () => {
        moving = false;
        for (const frame of document.querySelectorAll('iframe')) frame.contentWindow.postMessage('stop-motion','*');
      };
      addEventListener('message', event => { if (event.data === 'stop-motion') stopMotion(); });
    </script>`
}

async function effects(page) {
  const result = {}
  for (const frame of page.frames()) {
    const { who, clicks } = await frame.evaluate(() => ({
      who: new URLSearchParams(location.search).get('who') || 'top',
      clicks: document.querySelector('#go').dataset.clicks,
    }))
    assert.ok(!(who in result), `duplicate fixture frame ${who}`)
    result[who] = clicks
  }
  return result
}

for (const moving of ['owner', 'target']) {
  test(`forced frame clicks deliver with a continuously moving ${moving}`, {
    skip,
  }, async (t) => {
    const neoRun = buildNeoRun()
    const cleanups = []
    try {
      const origin = await serveFixture(cleanups, {
        pages: {
          '/force-top.html': document(moving, 'top'),
          '/force-child.html': document(moving, 'child'),
          '/force-leaf.html': document(moving, 'leaf'),
        },
      })
      const { playwright, cdp, ...browser } = await launchReference(
        cleanups,
        browserBinary,
        { cdp: true },
      )
      const [context] = playwright.contexts()
      const scripts = mkdtempSync(join(tmpdir(), 'neo-forced-frame-'))
      cleanups.push(() => rmSync(scripts, { recursive: true, force: true }))
      const expected = Array.from({ length: 3 }, () => ({
        ordinary: moving === 'target' ? 'TimeoutError' : 'unchecked',
        before: '0',
        forced: 'clicked',
        after: '1',
      }))
      for (const engine of ['Playwright', 'Neo']) {
        const page = await context.newPage()
        try {
          const session = await context.newCDPSession(page)
          await session.send('Emulation.setDeviceMetricsOverride', {
            width: 1000,
            height: 900,
            deviceScaleFactor: 1,
            mobile: false,
          })
          await session.send('Emulation.setFocusEmulationEnabled', {
            enabled: true,
          })
          await page.goto(`${origin}/force-top.html`)
          const { targetInfo } = await session.send('Target.getTargetInfo')
          const run = async (source, name) => {
            if (engine === 'Playwright')
              return runInPlaywright(context, targetInfo.targetId, source)
            const path = join(scripts, `${name}.js`)
            writeFileSync(path, source)
            return runInNeo(neoRun, browser.port, targetInfo.targetId, path)
          }
          const movingResult = await run(
            `const checkOrdinary=${moving === 'target'};\n${MOVING}`,
            'moving',
          )
          const movingEffect = await effects(page)
          await page.evaluate(() => stopMotion())
          const stoppedResult = await run(STOPPED, 'stopped')
          const stoppedEffect = await effects(page)
          const { targetInfos } = await cdp.send('Target.getTargets')
          const cross = targetInfos.find(
            (info) =>
              info.type === 'iframe' &&
              info.parentFrameId === targetInfo.targetId &&
              info.url.includes('localhost:'),
          )
          assert.ok(cross, 'the cross-site child uses an iframe target')
          assert.ok(
            targetInfos.some(
              (info) =>
                info.type === 'iframe' &&
                info.parentFrameId === cross.targetId &&
                new URL(info.url).pathname === '/force-leaf.html',
            ),
            'the mixed grandchild uses another iframe target',
          )
          assert.deepEqual(
            movingResult.value,
            expected,
            `${engine}: moving ${moving}`,
          )
          assert.deepEqual(movingEffect, {
            top: '0',
            same: '1',
            cross: '1',
            'same-nested': '0',
            'cross-nested': '1',
          })
          assert.deepEqual(
            stoppedResult.value,
            ['2', '2', '2'],
            `${engine}: ordinary clicks recover`,
          )
          assert.deepEqual(stoppedEffect, {
            top: '0',
            same: '2',
            cross: '2',
            'same-nested': '0',
            'cross-nested': '2',
          })
          t.diagnostic(
            `${engine}${engine === 'Playwright' ? ` ${playwrightVersion}` : ''}: moving ${moving}, ${moving === 'target' ? 'ordinary waits, ' : ''}forced clicks deliver, stopped ordinary clicks recover`,
          )
        } finally {
          await page.close()
        }
      }
    } finally {
      await runCleanups(cleanups)
    }
  })
}
