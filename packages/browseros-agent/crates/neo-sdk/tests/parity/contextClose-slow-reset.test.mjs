// context.close() when one of the browser's replies to Neo's resets is slow (review D1 P1): the
// close resolves within its budget, and no other reset waits for that reply. A CDP proxy between
// neo_run and the browser holds back the reply to the header reset (`Network.setExtraHTTPHeaders`
// with no headers), which the browser has applied already. Right after close() resolved, with
// that reply still held, the user's tab is online, sends no extra header and has the geolocation
// permission back to the profile's own, as Playwright's close leaves it
// (contextClose-scope.test.mjs); once the reply comes, it stays so.
//
// The run grants geolocation, which its close takes back last (`Tabs::release_run_scoped`): the
// permission reading `prompt` again on the test's own session tells the test the close went
// through, while the run's realm still lives (KEEP_ALIVE_MS).
//
// Runs under Node, not Bun, on the shared harness (harness.mjs).

import assert from 'node:assert/strict'
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import http from 'node:http'
import { createRequire } from 'node:module'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { test } from 'node:test'
import {
  browserBinary,
  buildNeoRun,
  connectCdp,
  eventually,
  launchBrowser,
  openTab,
  runCleanups,
  serveFixture,
  skip,
  startNeo,
} from './harness.mjs'

/** How long Neo's realm lives on after close() resolves. */
const KEEP_ALIVE_MS = 15_000

/**
 * A CDP proxy to `upstream` that holds back the reply to the first header reset after the run
 * set its header: `release()` lets it through.
 */
async function holdingProxy(cleanups, upstream) {
  const { ws: WebSocket, wsServer: WebSocketServer } = createRequire(
    import.meta.url,
  )(join(process.env.NEO_PARITY_PLAYWRIGHT, 'lib/utilsBundle.js'))
  let port
  let armed = false
  let held
  const sockets = new Set()
  const server = http.createServer((_request, response) =>
    response.end(
      JSON.stringify({ webSocketDebuggerUrl: `ws://127.0.0.1:${port}/proxy` }),
    ),
  )
  const wss = new WebSocketServer({ server })
  wss.on('connection', (down) => {
    const up = new WebSocket(upstream)
    const queue = []
    const calls = new Map()
    sockets.add(down)
    sockets.add(up)
    up.on('open', () => {
      for (const data of queue) up.send(data)
      queue.length = 0
    })
    down.on('message', (data) => {
      const text = data.toString()
      const message = JSON.parse(text)
      calls.set(message.id, message)
      if (
        message.method === 'Network.setExtraHTTPHeaders' &&
        Object.values(message.params.headers).includes('owned')
      )
        armed = true
      if (up.readyState === WebSocket.OPEN) up.send(text)
      else queue.push(text)
    })
    up.on('message', (data) => {
      const text = data.toString()
      const call = calls.get(JSON.parse(text).id)
      if (
        armed &&
        !held &&
        call?.method === 'Network.setExtraHTTPHeaders' &&
        Object.keys(call.params.headers).length === 0
      ) {
        held = () => {
          if (down.readyState === WebSocket.OPEN) down.send(text)
        }
        return
      }
      if (down.readyState === WebSocket.OPEN) down.send(text)
    })
    down.on('close', () => up.close())
    up.on('close', () => down.close())
    down.on('error', () => {})
    up.on('error', () => {})
  })
  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve))
  port = server.address().port
  const proxy = {
    port,
    get holding() {
      return Boolean(held)
    },
    release() {
      held?.()
      held = () => {}
    },
  }
  cleanups.push(async () => {
    proxy.release()
    for (const socket of sockets) socket.close()
    await new Promise((resolve) => wss.close(resolve))
    await new Promise((resolve) => server.close(resolve))
  })
  return proxy
}

test("a slow reply to one of close()'s resets leaves the tab online, as Playwright's close does", {
  skip,
  timeout: 120_000,
}, async () => {
  const neoRun = buildNeoRun()
  const cleanups = []
  try {
    const origin = await serveFixture(cleanups, {
      pages: {
        '/top.html': '<!doctype html><title>top</title>',
        '/probe': (request, response) => {
          response.setHeader('cache-control', 'no-store')
          response.end(request.headers['x-review'] ?? 'none')
        },
      },
    })
    const browser = await launchBrowser(cleanups, browserBinary)
    const cdp = await connectCdp(cleanups, browser.port)
    const { targetId, sessionId } = await openTab(cdp, `${origin}/top.html`)
    const { webSocketDebuggerUrl } = await (
      await fetch(`http://127.0.0.1:${browser.port}/json/version`)
    ).json()
    const proxy = await holdingProxy(cleanups, webSocketDebuggerUrl)
    const scripts = mkdtempSync(join(tmpdir(), 'neo-close-slow-reset-'))
    cleanups.push(() => rmSync(scripts, { recursive: true, force: true }))
    const script = join(scripts, 'slow-reset.js')
    writeFileSync(
      script,
      `await context.setExtraHTTPHeaders({ 'X-Review': 'owned' })
await context.grantPermissions(['geolocation'], { origin: ${JSON.stringify(origin)} })
await context.setGeolocation({ latitude: 10, longitude: 20 })
await context.setOffline(true)
await context.close()
await new Promise((resolve) => setTimeout(resolve, ${KEEP_ALIVE_MS}))
return 'closed'`,
    )
    const read = async () => {
      const reply = await cdp.send(
        'Runtime.evaluate',
        {
          expression: `(async () => ({
            online: navigator.onLine,
            permission: (await navigator.permissions.query({ name: 'geolocation' })).state,
            header: await fetch('/probe').then((response) => response.text(), () => 'offline'),
          }))()`,
          returnByValue: true,
          awaitPromise: true,
        },
        sessionId,
      )
      assert.equal(
        reply.exceptionDetails,
        undefined,
        JSON.stringify(reply.exceptionDetails),
      )
      return reply.result.value
    }
    const run = startNeo(neoRun, proxy.port, targetId, script)
    await run.connected
    await eventually(
      () => proxy.holding,
      'the header reset reply is held',
      20_000,
    )
    // The grant goes last in the close: back to the profile's own, the close went through.
    await eventually(
      async () => (await read()).permission === 'prompt',
      'the close let go of the grant',
      20_000,
    )
    const afterClose = await read()
    proxy.release()
    await eventually(
      async () => (await read()).online,
      'the tab stays online',
      5_000,
    )
    const afterReply = await read()
    const outcome = await run.outcome
    assert.equal(outcome.status, 'ok', JSON.stringify(outcome))
    assert.deepEqual(
      afterClose,
      { online: true, permission: 'prompt', header: 'none' },
      'right after close()',
    )
    assert.deepEqual(
      afterReply,
      { online: true, permission: 'prompt', header: 'none' },
      'once the reply came',
    )
    await cdp.send('Target.closeTarget', { targetId }).catch(() => {})
  } finally {
    await runCleanups(cleanups)
  }
})
