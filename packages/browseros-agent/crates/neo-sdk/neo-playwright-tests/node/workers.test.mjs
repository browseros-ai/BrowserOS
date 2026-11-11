// biome-ignore-all lint/suspicious/noUndeclaredEnvVars: Standalone browser integration harness.
/** Dedicated workers across startup, script evaluation, termination and navigation. */
import assert from 'node:assert/strict'
import { once } from 'node:events'
import fs from 'node:fs'
import http from 'node:http'
import os from 'node:os'
import path from 'node:path'
import test from 'node:test'
import { browserVersion, launchBrowser } from './browser.mjs'
import { openClaw } from './claw-loader.mjs'

test('dedicated workers preserve identity, values and terminal lifetimes', {
  skip: !process.env.NEO_E2E_BROWSER,
  timeout: 90_000,
}, async (t) => {
  const output = fs.mkdtempSync(
    path.join(process.env.NEO_NODE_OUTPUT_DIR || os.tmpdir(), 'workers-'),
  )
  t.diagnostic(`Evidence: ${output}`)
  const requests = []
  const server = http.createServer((req, res) => {
    requests.push({ url: req.url, host: req.headers.host })
    fs.writeFileSync(
      path.join(output, 'requests.json'),
      JSON.stringify(requests),
    )
    if (req.url === '/worker.js') {
      res.setHeader('content-type', 'application/javascript')
      res.end('self.initialValue = 42; console.log("worker startup");')
    } else {
      res.setHeader('content-type', 'text/html')
      res.end(
        req.url === '/start'
          ? '<script>window.worker = new Worker("/worker.js")</script>'
          : '<title>Workers</title>',
      )
    }
  })
  server.listen(0, '127.0.0.1')
  await once(server, 'listening')
  const url = `http://127.0.0.1:${server.address().port}`
  let browser
  let sdk
  try {
    const executable = process.env.NEO_E2E_BROWSER
    browser = await launchBrowser(executable, path.join(output, 'worker-0'), {
      launcherPid: process.pid,
    })
    sdk = await openClaw(browser.port, {
      timeoutMs: 70_000,
      stateDir: path.join(output, 'sdk'),
      browserosVersion: browserVersion(executable),
    })
    let page = await sdk.context.newPage()
    // Enumeration must work without a worker event listener. Navigation's page facts
    // initialize the synchronous cache; waits and events then reuse those objects.
    await page.goto(`${url}/start`)
    let [initial] = page.workers()
    assert(initial)
    assert(initial.url().endsWith('/worker.js'))
    assert.equal(await initial.evaluate(() => self.initialValue), 42)
    assert.equal(await initial.evaluate('1 + 1'), 2)
    assert.deepEqual(
      await initial.evaluate(
        async (arg) => ({ arg, value: await Promise.resolve(7) }),
        { nested: [1, 'two'] },
      ),
      { arg: { nested: [1, 'two'] }, value: 7 },
    )
    assert(Object.is(await initial.evaluate(() => -0), -0))
    assert(Number.isNaN(await initial.evaluate(() => NaN)))
    assert.equal(await initial.evaluate(() => 12n), 12n)
    await assert.rejects(
      initial.evaluate(() => {
        throw new Error('worker failure')
      }),
      /worker.evaluate.*worker failure/s,
    )
    assert.throws(() => initial.evaluateHandle(), /JS handles/)

    const consolePromise = initial.waitForEvent('console', { timeout: 3000 })
    await initial.evaluate(() => console.log('worker evaluation'))
    const message = await consolePromise
    assert.equal(message.text(), 'worker evaluation')
    assert.equal(message.worker(), initial)
    assert.equal(message.page(), page)

    const fromIframe = page.waitForEvent('worker', { timeout: 10000 })
    await page.evaluate(
      (source) => {
        const frame = document.createElement('iframe')
        frame.src = source
        document.body.append(frame)
      },
      `${url.replace('127.0.0.1', 'localhost')}/start`,
    )
    const iframeWorker = await fromIframe
    assert.equal(await iframeWorker.evaluate(() => self.initialValue), 42)
    const nestedCreated = page.waitForEvent('worker', { timeout: 3000 })
    await iframeWorker.evaluate(() => {
      self.child = new Worker(
        URL.createObjectURL(
          new Blob(['self.nested = 99'], { type: 'application/javascript' }),
        ),
      )
    })
    const nested = await nestedCreated
    assert.equal(await nested.evaluate(() => self.nested), 99)
    assert.equal(page.workers().length, 3)
    const iframeClosed = iframeWorker.waitForEvent('close', { timeout: 3000 })
    const nestedClosed = nested.waitForEvent('close', { timeout: 3000 })
    await page.evaluate(() => document.querySelector('iframe').remove())
    assert.equal(await iframeClosed, iframeWorker)
    assert.equal(await nestedClosed, nested)
    assert.deepEqual(page.workers(), [initial])

    // A fresh run must enumerate workers of an existing tab before any page operation.
    await sdk.finish()
    sdk = await openClaw(browser.port, {
      timeoutMs: 70_000,
      stateDir: path.join(output, 'sdk-adopted'),
      browserosVersion: browserVersion(executable),
    })
    page = sdk.context
      .pages()
      .find((candidate) => candidate.url() === `${url}/start`)
    assert(page)
    ;[initial] = page.workers()
    assert(initial, 'an untouched adopted page lists its existing worker')

    const closed = initial.waitForEvent('close', { timeout: 3000 })
    const pending = initial
      .evaluate(() => new Promise(() => {}))
      .catch((error) => error)
    // A same-document navigation preserves workers; a new document retires them.
    await page.evaluate(() => {
      location.hash = 'keep-worker'
    })
    assert.equal(page.workers()[0], initial)
    await page.goto(`${url}/empty`)
    assert.equal(await closed, initial)
    assert.match((await pending).message, /worker.evaluate.*closed/s)
    assert.deepEqual(page.workers(), [])

    await page.evaluate((source) => {
      setTimeout(() => {
        window.worker = new Worker(source)
      }, 50)
    }, `${url}/worker.js`)
    await new Promise((resolve) => setTimeout(resolve, 200))
    assert.equal(
      page.workers().length,
      1,
      'bookkeeping observes workers without user listeners or another host reply',
    )
    await page.evaluate(() => window.worker.terminate())
    assert.deepEqual(page.workers(), [])

    const created = page.waitForEvent('worker', { timeout: 3000 })
    const events = []
    page.on('worker', (worker) => events.push(worker))
    await page.evaluate(() => {
      window.worker = new Worker(
        URL.createObjectURL(
          new Blob(['self.value = 9'], { type: 'application/javascript' }),
        ),
      )
    })
    const worker = await created
    assert.equal(page.workers()[0], worker)
    assert.equal(await worker.evaluate(() => self.value), 9)
    assert.equal(events[0], worker)
    const terminated = worker.waitForEvent('close', { timeout: 3000 })
    await page.evaluate(() => window.worker.terminate())
    assert.equal(await terminated, worker)
    assert.deepEqual(page.workers(), [])
    await assert.rejects(worker.evaluate('1'), /worker.evaluate.*closed/s)

    const next = page.waitForEvent('worker', { timeout: 3000 })
    await page.goto(`${url}/start`)
    const closingWorker = await next
    const close = closingWorker.waitForEvent('close', { timeout: 3000 })
    const evaluating = closingWorker
      .evaluate(() => new Promise(() => {}))
      .catch((error) => error)
    await page.close()
    assert.equal(await close, closingWorker)
    assert.match((await evaluating).message, /worker.evaluate.*closed/s)
  } finally {
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
