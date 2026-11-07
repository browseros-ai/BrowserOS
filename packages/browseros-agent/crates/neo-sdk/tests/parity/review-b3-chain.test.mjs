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

for (const cancelMiddle of [false, true]) {
  test(`write chain keeps call order with ${cancelMiddle ? 'an aborted' : 'a live'} middle write`, {
    skip,
    timeout: 120_000,
  }, async (t) => {
    const value = await compareContentRun(
      t,
      `
      const frame=page.mainFrame();
      const controller=new AbortController();
      const first=frame.setContent('<h1 id="marker">First</h1>'+'<!--pad-->'.repeat(32768),{waitUntil:'commit'}).then(()=> 'resolved',errorCategory);
      const middle=frame.setContent('<h1 id="marker">Middle</h1>',{waitUntil:'commit',signal:controller.signal}).then(()=> 'resolved',errorCategory);
      const last=frame.setContent('<h1 id="marker">Last</h1>',{waitUntil:'commit'}).then(()=> 'resolved',errorCategory);
      ${cancelMiddle ? 'controller.abort();' : ''}
      const results=await Promise.all([first,middle,last]);
      return {results, text:await frame.locator('#marker').textContent()};
    `,
    )
    assert.equal(value.playwright.text, 'Last')
  })
}

test('a setContent timeout releases the write chain for a later write', {
  skip,
  timeout: 120_000,
}, async (t) => {
  const values = await compareContentRun(
    t,
    `
    const frame=page.mainFrame();
    const first=await frame.setContent('<p>First</p><script>const end=Date.now()+1200;while(Date.now()<end){}</script>',{timeout:300}).then(()=> 'resolved',errorCategory);
    await frame.setContent('<p id="recovered">Recovered</p>',{waitUntil:'commit',timeout:4000});
    return {first,text:await frame.locator('#recovered').textContent()};
  `,
  )
  assert.deepEqual(values.playwright, {
    first: 'TimeoutError',
    text: 'Recovered',
  })
})

test('waiting for content load holds neither input nor a later content write', {
  skip,
  timeout: 120_000,
}, async (t) => {
  const values = await compareContentRun(
    t,
    `
    const frame=page.mainFrame();
    const pending=frame.setContent('<button id="ready" onclick="this.textContent=1">Ready</button><img src="'+origin+'/hold?queue">',{timeout:5000}).then(()=> 'resolved',errorCategory);
    await frame.locator('#ready').waitFor();
    await frame.locator('#ready').click();
    const clicked=await frame.locator('#ready').textContent();
    await frame.setContent('<p id="last">Last</p>',{waitUntil:'commit',timeout:1500});
    await pending;
    return {clicked,text:await frame.locator('#last').textContent()};
  `,
  )
  assert.deepEqual(values.playwright, { clicked: '1', text: 'Last' })
})

test('a removed frame does not block content writes to its parent', {
  skip,
  timeout: 120_000,
}, async (t) => {
  const values = await compareContentRun(
    t,
    `
    const frame=page.frame('child');
    await frame.locator('#old').waitFor();
    const first=frame.setContent('<p id="written">Written</p><img src="'+origin+'/hold?remove">',{timeout:1500}).then(()=> 'resolved',errorCategory);
    await frame.locator('#written').waitFor();
    await page.locator('#remove').click();
    const removed=await first;
    await page.setContent('<p id="recovered">Recovered</p>',{waitUntil:'commit',timeout:1500});
    return {removed,text:await page.locator('#recovered').textContent()};
  `,
  )
  assert.deepEqual(values.playwright, { removed: 'Error', text: 'Recovered' })
})
