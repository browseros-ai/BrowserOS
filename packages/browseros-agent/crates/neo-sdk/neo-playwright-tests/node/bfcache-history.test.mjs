// biome-ignore-all lint/suspicious/noUndeclaredEnvVars: This standalone Node harness is not a Turbo task.
/** Desktop history restores must complete without new lifecycle events or disabling BFCache. */
import assert from 'node:assert/strict'
import { EventEmitter, once } from 'node:events'
import fs from 'node:fs'
import http from 'node:http'
import os from 'node:os'
import path from 'node:path'
import test from 'node:test'
import { Cdp } from '../js/browser.mjs'
import { browserVersion, launchBrowser } from './browser.mjs'
import { openClaw } from './claw-loader.mjs'

/** A second, read-only CDP session proves real restores instead of silently accepting reloads. */
async function observe(port, url) {
  const version = await (
    await fetch(`http://127.0.0.1:${port}/json/version`)
  ).json()
  const cdp = await Cdp.connect(version.webSocketDebuggerUrl, {
    timeoutMs: 5000,
  })
  try {
    const { targetInfos } = await cdp.send('Target.getTargets')
    const target = targetInfos.find((target) => target.url === url)
    assert(target)
    const { sessionId } = await cdp.send('Target.attachToTarget', {
      targetId: target.targetId,
      flatten: true,
    })
    await cdp.send('Page.enable', {}, sessionId)
    return {
      close: () => cdp.close(),
      // Subscribe synchronously before the SDK sends its command. The independent socket
      // may deliver after the SDK has already resolved, so the proof has its own deadline.
      restored(url) {
        return new Promise((resolve, reject) => {
          let offDisconnect = () => {}
          const unsubscribe = cdp.subscribe((event) => {
            if (
              event.sessionId === sessionId &&
              event.method === 'Page.frameNavigated' &&
              event.params.type === 'BackForwardCacheRestore' &&
              event.params.frame.url === url
            )
              finish(undefined, event.params)
          })
          const finish = (error, event) => {
            clearTimeout(timer)
            unsubscribe()
            offDisconnect()
            if (error) reject(error)
            else resolve(event)
          }
          const timer = setTimeout(
            () => finish(new Error('No BFCache restore observed')),
            5000,
          )
          offDisconnect = cdp.onDisconnect((error) => finish(error))
        })
      },
      async marker() {
        const { result } = await cdp.send(
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
    await cdp.close()
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
  const requests = new EventEmitter()
  let slowResponse
  const server = http.createServer((req, res) => {
    res.setHeader('content-type', 'text/html')
    if (req.url === '/slow.js') {
      slowResponse = res
      requests.emit('slow')
      return
    }
    if (req.url === '/loading-child')
      return res.end('<script src="/slow.js"></script>')
    if (req.url === '/restore-child')
      return res.end(`<script>addEventListener('pageshow', event => {
      if (event.persisted) {
        const child = document.createElement('iframe'); child.src = '/loading-child';
        document.body.appendChild(child);
      }
    })</script>`)
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
        const started = Date.now()
        const [restored, response] = await Promise.all([
          observer.restored(`${url}/${entry}`),
          page[method]({ waitUntil, timeout: 3000 }),
        ])
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
    // A restored parent's states cannot make a new pageshow child ready. Its stalled
    // resource is a real website wait, unlike Neo missing the parent's restore event.
    await page.goto(`${url}/restore-child`)
    await page.goto(`${url}/b`)
    const requested = once(requests, 'slow', {
      signal: AbortSignal.timeout(5000),
    })
    const childRestore = observer.restored(`${url}/restore-child`)
    const [error] = await Promise.all([
      page
        .goBack({ waitUntil: 'networkidle', timeout: 1000 })
        .catch((error) => error),
      childRestore,
      requested,
    ])
    assert.equal(error.code, 'wait_timeout')
    assert.equal(error.owner, 'website')
    slowResponse.end('')
    fs.writeFileSync(
      path.join(output, 'proof.json'),
      JSON.stringify(
        {
          passed,
          newChildWait: { owner: error.owner, code: error.code },
        },
        null,
        2,
      ),
    )
  } finally {
    await observer?.close()
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
