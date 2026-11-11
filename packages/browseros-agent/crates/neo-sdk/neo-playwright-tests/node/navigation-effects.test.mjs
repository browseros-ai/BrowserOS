// biome-ignore-all lint/suspicious/noUndeclaredEnvVars: This standalone Node harness is not a Turbo task.
/** Navigation effects across the renderer, cached frame facts, response identity and close. */
import assert from 'node:assert/strict'
import { EventEmitter, once } from 'node:events'
import fs from 'node:fs'
import http from 'node:http'
import os from 'node:os'
import path from 'node:path'
import test from 'node:test'
import { browserVersion, launchBrowser } from './browser.mjs'
import { openClaw } from './claw-loader.mjs'

test('navigation effects preserve deadlines, facts, responses and close semantics', {
  skip: !process.env.NEO_E2E_BROWSER,
  timeout: 120_000,
}, async (t) => {
  const output = fs.mkdtempSync(
    path.join(
      process.env.NEO_NODE_OUTPUT_DIR || os.tmpdir(),
      'navigation-effects-',
    ),
  )
  const requests = new EventEmitter()
  const order = []
  const server = http.createServer((req, res) => {
    requests.emit(req.url)
    res.setHeader('content-type', 'text/html')
    if (req.url === '/hang' || req.url === '/slow.js') return
    if (req.url === '/loading')
      return res.end('<script src="/slow.js"></script>')
    if (req.url === '/a')
      return res.end('<script>setTimeout(() => location="/c", 10)</script>')
    if (req.url === '/b' || req.url === '/c') {
      order.push(`started ${req.url}`)
      setTimeout(() => {
        order.push(`finished ${req.url}`)
        res.end('Done')
      }, 1000)
      return
    }
    if (req.url === '/parent')
      return res.end('<iframe src="/loading"></iframe>')
    res.end('<title>Ready</title><h1>Ready</h1>')
  })
  server.listen(0, '127.0.0.1')
  await once(server, 'listening')
  const url = `http://127.0.0.1:${server.address().port}`
  const executable = process.env.NEO_E2E_BROWSER
  let browser, sdk
  const passed = []
  const record = (label) => {
    passed.push(label)
    fs.writeFileSync(
      path.join(output, 'proof.json'),
      JSON.stringify({ passed }, null, 2),
    )
  }
  t.diagnostic(`Evidence: ${output}`)
  try {
    browser = await launchBrowser(executable, path.join(output, 'worker'), {
      launcherPid: process.pid,
    })
    sdk = await openClaw(browser.port, {
      timeoutMs: 60000,
      stateDir: path.join(output, 'sdk-state'),
      browserosVersion: browserVersion(executable),
    })
    const page = await sdk.context.newPage()
    await page.goto(url)
    assert.equal(await page.goto(`${url}/#fragment`), null)
    await page.evaluate(() => {
      history.pushState({}, '', '/state')
      const frame = document.createElement('iframe')
      document.body.appendChild(frame)
    })
    assert.equal(page.url(), `${url}/state`)
    assert.equal(page.frames().length, 2)
    await page.title()
    assert.equal(page.url(), `${url}/state`)
    record(
      'fragment goto and evaluated history/frame effects refresh synchronous facts',
    )
    const child = page.frames()[1]
    const childResponse = await child.goto(`${url}/child`)
    assert.equal(childResponse.status(), 200)
    assert.equal(childResponse.frame(), child)
    assert.equal(child.url(), `${url}/child`)
    record('child goto watches and reads the selected document')
    const hung = once(requests, '/hang')
    const first = page
      .goto(`${url}/hang`, { timeout: 3000 })
      .catch((error) => error)
    await hung
    assert.equal(
      (await page.goto(`${url}/next`, { timeout: 3000 })).status(),
      200,
    )
    assert.match((await first).message, /ERR_ABORTED|interrupted/)
    record('replacement goto unblocks a stalled navigation')
    const loading = once(requests, '/slow.js')
    const original = page.goto(`${url}/loading`, { timeout: 3000 })
    await loading
    await new Promise((resolve) => setTimeout(resolve, 100))
    await page.goto(`${url}/next`)
    assert.equal((await original).status(), 200)
    assert.equal((await original).url(), `${url}/loading`)
    record('a later navigation cannot replace the selected response')
    const c = once(requests, '/c')
    await page.goto(`${url}/a`)
    await c
    await page.waitForTimeout(100)
    await page.goto(`${url}/b`)
    assert(order.indexOf('started /b') < order.indexOf('finished /c'))
    assert.equal(page.url(), `${url}/b`)
    record(
      'supplemental reads do not wait for a later pending renderer navigation',
    )
    await assert.rejects(
      page.goto(`${url}/parent`, { waitUntil: 'networkidle', timeout: 1000 }),
      (error) => error.name === 'TimeoutError',
    )
    record('networkidle includes a child with a stalled resource')
    const waiter = page.waitForEvent('download').catch((error) => error)
    await Promise.all([page.close(), page.close()])
    await page.close()
    assert.match(
      (await waiter).message,
      /Target page, context or browser has been closed/,
    )
    assert.equal(page.isClosed(), true)
    record('close is idempotent and rejects other event waiters')
  } finally {
    // Host and browser own separate resources; each cleanup must run if the other fails.
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
