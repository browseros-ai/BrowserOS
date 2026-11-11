// biome-ignore-all lint/suspicious/noUndeclaredEnvVars: This standalone Node harness is not a Turbo task.
/** Desktop history restores must complete without new lifecycle events or disabling BFCache. */
import assert from 'node:assert/strict'
import { once } from 'node:events'
import fs from 'node:fs'
import http from 'node:http'
import os from 'node:os'
import path from 'node:path'
import test from 'node:test'
import { browserVersion, launchBrowser } from './browser.mjs'
import { openClaw } from './claw-loader.mjs'

/** A second, read-only CDP session proves real restores instead of silently accepting reloads. */
async function observe(port, url) {
  const version = await (
    await fetch(`http://127.0.0.1:${port}/json/version`)
  ).json()
  const socket = new WebSocket(version.webSocketDebuggerUrl)
  await once(socket, 'open', { signal: AbortSignal.timeout(5000) })
  let sequence = 0
  const pending = new Map()
  const events = []
  socket.addEventListener('message', ({ data }) => {
    const reply = JSON.parse(data)
    if (!reply.id) {
      if (reply.method === 'Page.frameNavigated') events.push(reply.params)
      return
    }
    const request = pending.get(reply.id)
    if (!request) return
    pending.delete(reply.id)
    clearTimeout(request.timer)
    if (reply.error) request.reject(new Error(reply.error.message))
    else request.resolve(reply.result)
  })
  const close = () => {
    for (const request of pending.values()) {
      clearTimeout(request.timer)
      request.reject(new Error('CDP observer closed'))
    }
    pending.clear()
    socket.close()
  }
  const send = (method, params = {}, sessionId) =>
    new Promise((resolve, reject) => {
      const id = ++sequence
      const timer = setTimeout(() => {
        pending.delete(id)
        reject(new Error(`CDP observer timed out: ${method}`))
      }, 5000)
      pending.set(id, { resolve, reject, timer })
      socket.send(JSON.stringify({ id, method, params, sessionId }))
    })
  try {
    const { targetInfos } = await send('Target.getTargets')
    const target = targetInfos.find((target) => target.url === url)
    assert(target)
    const { sessionId } = await send('Target.attachToTarget', {
      targetId: target.targetId,
      flatten: true,
    })
    await send('Page.enable', {}, sessionId)
    return {
      events,
      close,
      async marker() {
        const { result } = await send(
          'Runtime.evaluate',
          {
            expression: 'globalThis.cacheMarker',
            returnByValue: true,
          },
          sessionId,
        )
        return result.value
      },
    }
  } catch (error) {
    close()
    throw error
  }
}

test('desktop BFCache back and forward reach every load state', {
  skip: !process.env.NEO_E2E_BROWSER,
  timeout: 90_000,
}, async (t) => {
  const output = fs.mkdtempSync(
    path.join(
      process.env.NEO_NODE_OUTPUT_DIR || os.tmpdir(),
      'bfcache-history-',
    ),
  )
  const server = http.createServer((req, res) => {
    res.setHeader('content-type', 'text/html')
    res.end(
      `<title>${req.url}</title><script>globalThis.cacheMarker = crypto.randomUUID()</script>`,
    )
  })
  server.listen(0, '127.0.0.1')
  await once(server, 'listening')
  const url = `http://127.0.0.1:${server.address().port}`
  const executable = process.env.NEO_E2E_BROWSER
  let browser, sdk, observer
  const passed = []
  t.diagnostic(`Evidence: ${output}`)
  try {
    // The normal desktop-binary launcher keeps BFCache enabled. Every step also checks the
    // protocol's restore type and a retained JS marker, so a reload cannot satisfy this test.
    browser = await launchBrowser(executable, path.join(output, 'worker'), {
      launcherPid: process.pid,
    })
    sdk = await openClaw(browser.port, {
      timeoutMs: 60000,
      stateDir: path.join(output, 'sdk-state'),
      browserosVersion: browserVersion(executable),
    })
    const page = await sdk.context.newPage()
    await page.goto(`${url}/a`)
    observer = await observe(browser.port, `${url}/a`)
    const markers = { a: await observer.marker() }
    await page.goto(`${url}/b`)
    markers.b = await observer.marker()
    assert.notEqual(markers.a, markers.b)
    for (const waitUntil of [
      'load',
      'domcontentloaded',
      'commit',
      'networkidle',
    ]) {
      for (const [method, entry] of [
        ['goBack', 'a'],
        ['goForward', 'b'],
      ]) {
        const after = observer.events.length
        const started = Date.now()
        const response = await page[method]({ waitUntil, timeout: 3000 })
        const restored = observer.events
          .slice(after)
          .find(
            (event) =>
              event.type === 'BackForwardCacheRestore' &&
              event.frame.url === `${url}/${entry}`,
          )
        assert(
          restored,
          `${method}(${waitUntil}) must actually restore from BFCache`,
        )
        assert.equal(page.url(), `${url}/${entry}`)
        assert.equal(await observer.marker(), markers[entry])
        if (response) {
          assert.equal(response.status(), 200)
          assert.equal(response.url(), `${url}/${entry}`)
        }
        passed.push({
          method,
          waitUntil,
          elapsedMs: Date.now() - started,
          loader: restored.frame.loaderId,
        })
        fs.writeFileSync(
          path.join(output, 'proof.json'),
          JSON.stringify({ passed }, null, 2),
        )
      }
    }
    assert.equal(passed.length, 8)
  } finally {
    observer?.close()
    try {
      await sdk?.finish()
    } finally {
      try {
        await browser?.close()
      } finally {
        server.closeAllConnections()
        await new Promise((resolve) => server.close(resolve))
      }
    }
  }
})
