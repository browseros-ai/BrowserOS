import assert from 'node:assert/strict'
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { createServer } from 'node:http'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { test } from 'node:test'
import {
  browserBinary,
  buildNeoRun,
  chromium,
  errorCategory,
  launchBrowser,
  runCleanups,
  skip,
  startNeo,
} from './harness.mjs'

const isolate = `<script>new MutationObserver(()=>document.querySelector('browseros-neo-takeover')?.remove()).observe(document.documentElement,{childList:true,subtree:true})</script>`

// The image remains pending until this test releases it. No assertion depends on
// browser timing or a fixed server delay.
async function compareContentRun(t, source) {
  const cleanups = [],
    held = new Set()
  const release = () => {
    for (const response of held)
      response.end(
        '<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"/>',
      )
    held.clear()
  }
  try {
    const server = createServer((request, response) => {
      if (request.url.startsWith('/hold')) {
        response.setHeader('content-type', 'image/svg+xml')
        held.add(response)
        response.on('close', () => held.delete(response))
        return
      }
      response.setHeader('content-type', 'text/html')
      const pages = {
        '/main': `<button id="remove" onclick="document.querySelector('#child').remove()">Remove</button><button id="navigate" onclick="document.querySelector('#child').src='/next'">Navigate</button><iframe id="child" name="child" src="/child"></iframe>${isolate}`,
        '/child': `<p id="old">Old child</p>${isolate}`,
        '/next': `<title>Next</title><p id="next">Next child</p>${isolate}`,
      }
      response.end(pages[request.url] ?? '<p>New page</p>')
    })
    await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve))
    cleanups.push(async () => {
      release()
      server.closeAllConnections()
      await new Promise((resolve) => server.close(resolve))
    })
    const origin = `http://127.0.0.1:${server.address().port}`
    const browser = await launchBrowser(cleanups, browserBinary)
    const pw = await chromium.connectOverCDP(`http://127.0.0.1:${browser.port}`)
    cleanups.push(() => pw.close())
    const [context] = pw.contexts()
    const dir = mkdtempSync(join(tmpdir(), 'neo-b3-review-'))
    cleanups.push(() => rmSync(dir, { recursive: true, force: true }))
    const script = `const origin=${JSON.stringify(origin)};const errorCategory=${errorCategory.toString()};${source}`
    const values = {}
    for (const engine of ['playwright', 'neo']) {
      const page = await context.newPage()
      try {
        await page.goto(origin + '/main')
        if (engine === 'playwright')
          values[engine] = await new (async () => {}).constructor(
            'page',
            script,
          )(page)
        else {
          const session = await context.newCDPSession(page)
          const { targetInfo } = await session.send('Target.getTargetInfo')
          await session.detach()
          const file = join(dir, 'run.js')
          writeFileSync(file, script)
          const running = startNeo(
            buildNeoRun(),
            browser.port,
            targetInfo.targetId,
            file,
          )
          const outcome = await running.outcome
          assert.equal(outcome.status, 'ok', JSON.stringify(outcome))
          values[engine] = outcome.value
        }
      } finally {
        release()
        if (!page.isClosed()) await page.close()
      }
    }
    t.diagnostic(JSON.stringify(values))
    assert.deepEqual(values.neo, values.playwright, 'public outcome')
    return values
  } finally {
    release()
    await runCleanups(cleanups)
  }
}

for (const action of ['remove', 'navigate'])
  test(`review: setContent survives or rejects a child ${action} as the pin does`, {
    skip,
    timeout: 120_000,
  }, async (t) => {
    await compareContentRun(
      t,
      `
    const frame=page.frame('child');await frame.locator('#old').waitFor();
    const pending=frame.setContent('<p id="written">Written</p><img src="'+origin+'/hold">',{timeout:1500}).then(()=> 'resolved',errorCategory);
    await frame.locator('#written').waitFor();await page.locator('#${action}').click();
    return await pending;
  `,
    )
  })

test('review: load-state waits observe the document reopened by setContent', {
  skip,
  timeout: 120_000,
}, async (t) => {
  await compareContentRun(
    t,
    `
    const frame=page.mainFrame();await frame.waitForLoadState('load');
    await frame.setContent('<p id="written">Written</p><img id="asset" src="'+origin+'/hold" data-loaded="0" onload="this.dataset.loaded=1">',{waitUntil:'commit'});
    const wait=await frame.waitForLoadState('load',{timeout:300}).then(()=> 'resolved',errorCategory);
    return {wait,loaded:await frame.locator('#asset').getAttribute('data-loaded')};
  `,
  )
})

test('review: waitForTimeout rejects when its page closes', {
  skip,
  timeout: 120_000,
}, async (t) => {
  await compareContentRun(
    t,
    `
    const pending=page.waitForTimeout(10000).then(()=> 'resolved',errorCategory);
    await page.close();return await pending;
  `,
  )
})

test('review: waitForTimeout keeps Node numeric delay semantics', {
  skip,
  timeout: 120_000,
}, async (t) => {
  await compareContentRun(
    t,
    `
    const result=[];
    for(const delay of [0,NaN,Infinity,2147483648])
      result.push(await page.waitForTimeout(delay).then(()=> 'resolved',errorCategory));
    return result;
  `,
  )
})
