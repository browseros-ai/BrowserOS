/**
 * Review B2b-R1 (#3228): `dispatchEvent` on an `aria-ref` adopts its match into the main world
 * for the call (neo-runtime `element_ops.rs` `ref_attempt`). A timeout that wins while
 * Chromium's `DOM.resolveNode` reply is still on its way must not leave the adopted object alive
 * until the run ends: the call's own object group is released. The order is forced in-test: a
 * CDP proxy holds that reply until Neo has released the group. Timers only bound failures and
 * set the call's deadline.
 */
import assert from 'node:assert/strict'
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import http from 'node:http'
import { createRequire } from 'node:module'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'
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

const require = createRequire(import.meta.url)
const { ws: WebSocket, wsServer: WebSocketServer } = require(
  join(
    process.env.NEO_PARITY_PLAYWRIGHT ??
      dirname(require.resolve('playwright-core/package.json')),
    'lib/utilsBundle.js',
  ),
)

function deferred() {
  let resolve
  const promise = new Promise((settle) => {
    resolve = settle
  })
  return { promise, resolve }
}

function bounded(promise, what) {
  return Promise.race([
    promise,
    new Promise((_, reject) => {
      setTimeout(
        () => reject(new Error(`timed out waiting for ${what}`)),
        15_000,
      ).unref()
    }),
  ])
}

/**
 * A CDP proxy between Neo and the browser on `port`. It holds the reply to Neo's
 * `DOM.resolveNode` of the node whose `DOM.describeNode` named `target`, records the object
 * groups Neo releases, and sends commands of its own on the same connection.
 */
async function holdingProxy(cleanups, port) {
  const proxy = {
    requests: new Map(),
    own: new Map(),
    next: 1_000_000,
    held: [],
    releasedGroups: [],
    adopted: deferred(),
    backend: null,
  }
  const key = (message) => `${message.sessionId}:${message.id}`
  const server = http.createServer(async (request, response) => {
    const upstream = await fetch(`http://127.0.0.1:${port}${request.url}`)
    const own = `127.0.0.1:${server.address().port}`
    response.writeHead(upstream.status, {
      'content-type':
        upstream.headers.get('content-type') ?? 'application/json',
    })
    response.end(
      (await upstream.text())
        .replaceAll(`127.0.0.1:${port}`, own)
        .replaceAll(`localhost:${port}`, own),
    )
  })
  const wss = new WebSocketServer({
    noServer: true,
    maxPayload: 512 * 1024 * 1024,
  })
  server.on('upgrade', (request, socket, head) =>
    wss.handleUpgrade(request, socket, head, (neo) => {
      const upstream = new WebSocket(`ws://127.0.0.1:${port}${request.url}`, {
        maxPayload: 512 * 1024 * 1024,
        perMessageDeflate: false,
      })
      const queue = []
      const send = (text) =>
        upstream.readyState === 1 ? upstream.send(text) : queue.push(text)
      proxy.inject = (method, params, sessionId) => {
        const id = ++proxy.next
        const reply = deferred()
        proxy.own.set(`${sessionId}:${id}`, reply)
        send(JSON.stringify({ id, sessionId, method, params }))
        return bounded(reply.promise, method)
      }
      proxy.release = () => {
        for (const text of proxy.held.splice(0)) neo.send(text)
      }
      upstream.on('open', () => {
        for (const text of queue.splice(0)) upstream.send(text)
      })
      neo.on('message', (data) => {
        const text = data.toString()
        const message = JSON.parse(text)
        proxy.requests.set(key(message), message)
        if (message.method === 'Runtime.releaseObjectGroup')
          proxy.releasedGroups.push(message.params.objectGroup)
        send(text)
      })
      upstream.on('message', (data) => {
        const text = data.toString()
        const message = JSON.parse(text)
        const own = proxy.own.get(key(message))
        if (own) {
          proxy.own.delete(key(message))
          own.resolve(message)
          return
        }
        const sent = proxy.requests.get(key(message))
        proxy.requests.delete(key(message))
        if (
          sent?.method === 'DOM.describeNode' &&
          message.result?.node?.attributes?.includes('target')
        )
          proxy.backend = message.result.node.backendNodeId
        if (
          sent?.method === 'DOM.resolveNode' &&
          proxy.backend !== null &&
          sent.params.backendNodeId === proxy.backend &&
          message.result?.object?.objectId
        ) {
          proxy.held.push(text)
          proxy.adopted.resolve({
            objectId: message.result.object.objectId,
            sessionId: message.sessionId,
            group: sent.params.objectGroup,
          })
          return
        }
        neo.send(text)
      })
      neo.on('close', () => upstream.close())
      upstream.on('close', () => neo.close())
    }),
  )
  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve))
  cleanups.push(() => {
    proxy.release?.()
    for (const client of wss.clients) client.close()
    server.closeAllConnections?.()
    server.close()
  })
  proxy.port = server.address().port
  return proxy
}

test('dispatchEvent on an aria-ref releases the adoption its timeout cut off', {
  skip,
  timeout: 120_000,
}, async () => {
  const cleanups = []
  try {
    const origin = await serveFixture(cleanups, {
      pages: {
        '/adopt.html': `<button id="target">Target</button>
<script>window.events = []; document.addEventListener('probe-event', (event) => window.events.push(event.target.id))</script>`,
      },
    })
    const browser = await launchBrowser(cleanups, browserBinary)
    const cdp = await connectCdp(cleanups, browser.port)
    const { targetId, sessionId } = await openTab(cdp, `${origin}/adopt.html`)
    const read = async (expression) =>
      (
        await cdp.send(
          'Runtime.evaluate',
          { expression, returnByValue: true },
          sessionId,
        )
      ).result.value
    const proxy = await holdingProxy(cleanups, browser.port)
    const scripts = mkdtempSync(join(tmpdir(), 'neo-adoption-review-'))
    cleanups.push(() => rmSync(scripts, { recursive: true, force: true }))
    const script = join(scripts, 'adopt.js')
    writeFileSync(
      script,
      `
      const yaml = await page.locator('#target').ariaSnapshot({ mode: 'ai' })
      const ref = /\\[ref=([a-z0-9]+)\\]/.exec(yaml)[1]
      const outcome = await page.locator('aria-ref=' + ref)
        .dispatchEvent('probe-event', {}, { timeout: 1500 })
        .then(() => 'resolved', (error) => error.name)
      await page.evaluate(() => { window.callEnded = true })
      await page.waitForFunction(() => window.finishRun)
      return outcome
    `,
    )
    const neo = startNeo(buildNeoRun(), proxy.port, targetId, script)
    await neo.connected
    const adopted = await bounded(
      proxy.adopted.promise,
      'the held DOM.resolveNode reply',
    )
    // The call's deadline passes while that reply is held.
    await eventually(
      async () => (await read('window.callEnded')) === true,
      'the call ended',
    )
    await eventually(
      () => proxy.releasedGroups.includes(adopted.group),
      `Neo released ${adopted.group}`,
    )
    proxy.release()
    const inspected = await proxy.inject(
      'Runtime.callFunctionOn',
      {
        objectId: adopted.objectId,
        functionDeclaration: 'function () { return this.id }',
        returnByValue: true,
      },
      adopted.sessionId,
    )
    assert.ok(
      inspected.error,
      `the adopted object outlived its call: ${JSON.stringify(inspected)}`,
    )
    await read('window.finishRun = true')
    const outcome = await neo.outcome
    assert.equal(outcome.status, 'ok', JSON.stringify(outcome))
    assert.equal(outcome.value, 'TimeoutError')
    assert.deepEqual(await read('window.events'), [])
  } finally {
    await runCleanups(cleanups)
  }
})
