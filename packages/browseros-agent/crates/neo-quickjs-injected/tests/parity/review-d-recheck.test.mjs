import assert from 'node:assert/strict'
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { createServer } from 'node:http'
import { createRequire } from 'node:module'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { test } from 'node:test'
import { withFrames } from './frames.mjs'
import {
  browserBinary,
  buildNeoRun,
  launchReference,
  runCleanups,
  skip,
  startNeo,
} from './harness.mjs'

const WebSocket = skip
  ? null
  : createRequire(join(process.env.NEO_PARITY_PLAYWRIGHT, 'package.json'))('ws')

const until = async (what, check) => {
  const deadline = Date.now() + 10000
  while (!check()) {
    assert.ok(Date.now() < deadline, what)
    await new Promise((resolve) => setTimeout(resolve, 10))
  }
}

test('review D recheck: foreign nodes cannot alias a local option across sessions', {
  skip,
  timeout: 120000,
}, async (t) => {
  await withFrames(t, {}, async (run) => {
    const { playwright, neo } = await run({
      source: `const local=page.locator('#size');
        const same=await page.frameLocator('#same').locator('#size option[value=m]').elementHandle();
        const remote=await page.frameLocator('#login-iframe').locator('#size option[value=m]').elementHandle();
        const other=await page.context().newPage();
        try {
          await other.goto(page.url());
          const tab=await other.locator('#size option[value=m]').elementHandle();
          const out={};
          for(const [name,handle] of [['same',same],['remote',remote],['tab',tab]]){
            await local.selectOption('s');
            const error=await local.selectOption(handle,{timeout:300}).then(()=> 'resolved',e=>e.name);
            out[name]={error,selected:await local.inputValue()};
          }
          return out;
        } finally { await other.close() }`,
    })
    t.diagnostic(
      JSON.stringify({ playwright: playwright.result, neo: neo.result }),
    )
    assert.deepEqual(playwright.result, {
      value: {
        same: { error: 'resolved', selected: 'm' },
        remote: { error: 'TimeoutError', selected: 's' },
        tab: { error: 'TimeoutError', selected: 's' },
      },
    })
    // Deviation (DECIDED med-pw-dom, D re-check): a handle of another session (an
    // out-of-process frame's, another tab's) fails at once with an Error in Neo, where
    // Playwright retries its failed adoption until the timeout. Neither selects anything.
    assert.deepEqual(neo.result, {
      value: {
        same: { error: 'resolved', selected: 'm' },
        remote: { error: 'Error', selected: 's' },
        tab: { error: 'Error', selected: 's' },
      },
    })
  })
})

test('review D recheck: adopted objects are released before run end', {
  skip,
  timeout: 120000,
}, async (t) => {
  const cleanups = []
  const groups = new Map()
  const sockets = new Set()
  let releaseMarker = () => {}
  let markerReached = false
  let running
  try {
    const html = `<select id="s"><option value="a">A</option><option id="chosen" value="b">B</option></select><select><option id="gone" value="g">G</option></select><iframe srcdoc="<option id='foreign' value='f'>F</option>"></iframe>`
    const fixture = createServer((request, response) => {
      if (request.url === '/marker') {
        markerReached = true
        releaseMarker = () => response.end('released')
      } else {
        response.setHeader('content-type', 'text/html')
        response.end(html)
      }
    })
    await new Promise((resolve) => fixture.listen(0, '127.0.0.1', resolve))
    cleanups.push(async () => {
      releaseMarker()
      fixture.closeAllConnections()
      await new Promise((resolve) => fixture.close(resolve))
    })
    const { playwright: pw, ...browser } = await launchReference(
      cleanups,
      browserBinary,
    )
    const [context] = pw.contexts()
    const page = await context.newPage()
    await page.goto(`http://127.0.0.1:${fixture.address().port}/`)
    const version = await (
      await fetch(`http://127.0.0.1:${browser.port}/json/version`)
    ).json()
    const proxy = createServer((_request, response) => {
      response.setHeader('content-type', 'application/json')
      response.end(
        JSON.stringify({
          ...version,
          webSocketDebuggerUrl: `ws://127.0.0.1:${proxy.address().port}/browser`,
        }),
      )
    })
    const wss = new WebSocket.WebSocketServer({ server: proxy })
    let connection = 0
    wss.on('connection', (client) => {
      const current = ++connection
      const upstream = new WebSocket(version.webSocketDebuggerUrl)
      const pending = new Map(),
        queued = []
      sockets.add(client)
      sockets.add(upstream)
      upstream.on('open', () => {
        for (const message of queued) upstream.send(message)
        queued.length = 0
      })
      client.on('message', (bytes) => {
        const text = bytes.toString(),
          message = JSON.parse(text)
        const group = message.params?.objectGroup
        if (group?.startsWith('neo-select-')) {
          const key = `${current}/${message.sessionId}/${group}`
          const state = groups.get(key) ?? {
            group,
            session: message.sessionId,
            resolved: 0,
            released: false,
          }
          groups.set(key, state)
          pending.set(message.id, { state, method: message.method })
        }
        if (upstream.readyState === WebSocket.OPEN) upstream.send(text)
        else queued.push(text)
      })
      upstream.on('message', (bytes) => {
        const text = bytes.toString(),
          message = JSON.parse(text),
          entry = pending.get(message.id)
        if (entry) {
          pending.delete(message.id)
          if (
            entry.method === 'DOM.resolveNode' &&
            message.result?.object?.objectId
          )
            entry.state.resolved++
          if (entry.method === 'Runtime.releaseObjectGroup')
            entry.state.released = !message.error
        }
        if (client.readyState === WebSocket.OPEN) client.send(text)
      })
      for (const socket of [client, upstream]) {
        socket.on('error', () => {})
        socket.on('close', () => {
          sockets.delete(socket)
          if (client.readyState === WebSocket.OPEN) client.close()
          if (upstream.readyState === WebSocket.OPEN) upstream.close()
        })
      }
    })
    await new Promise((resolve) => proxy.listen(0, '127.0.0.1', resolve))
    cleanups.push(async () => {
      for (const socket of sockets) socket.terminate()
      await new Promise((resolve) => wss.close(resolve))
      await new Promise((resolve) => proxy.close(resolve))
    })
    const session = await context.newCDPSession(page)
    const { targetInfo } = await session.send('Target.getTargetInfo')
    await session.detach()
    const dir = mkdtempSync(join(tmpdir(), 'neo-d-groups-'))
    cleanups.push(() => rmSync(dir, { recursive: true, force: true }))
    const file = join(dir, 'run.js')
    writeFileSync(
      file,
      `const handle=await page.$('#chosen');
      await page.selectOption('#s',handle);await page.selectOption('#s',[handle,handle]);
      const gone=await page.$('#gone');await page.evaluate(()=>document.querySelector('#gone').remove());
      const timeout=await page.selectOption('#s',gone,{timeout:180}).then(()=> 'resolved',e=>e.name);
      await page.frameLocator('iframe').locator('#foreign').waitFor({state:'attached'});
      const foreign=await page.frameLocator('iframe').locator('#foreign').elementHandle();
      const refused=await page.selectOption('#s',foreign,{timeout:180}).then(()=> 'resolved',e=>e.name);
      const selected=await page.inputValue('#s');
      await page.evaluate(()=>fetch('/marker').then(r=>r.text()));return {timeout,refused,selected};`,
    )
    running = startNeo(
      buildNeoRun(),
      proxy.address().port,
      targetInfo.targetId,
      file,
    )
    await running.connected
    await until('script reaches live-run marker', () => markerReached)
    await until(
      'every select group released while run is still live',
      () => groups.size > 0 && [...groups.values()].every((g) => g.released),
    )
    assert.ok(
      [...groups.values()].some((g) => g.resolved > 0),
      'observed real adopted objects',
    )
    const beforeRunEnd = [...groups.values()]
    t.diagnostic(JSON.stringify({ beforeRunEnd }))
    releaseMarker()
    const outcome = await running.outcome
    assert.equal(outcome.status, 'ok', JSON.stringify(outcome))
    assert.deepEqual(outcome.value, {
      timeout: 'TimeoutError',
      refused: 'TimeoutError',
      selected: 'b',
    })
  } finally {
    releaseMarker()
    if (running) await running.outcome.catch(() => {})
    await runCleanups(cleanups)
  }
})
