// biome-ignore-all lint/suspicious/noUndeclaredEnvVars: This standalone Node harness is not a Turbo task.
/** Lifecycle payloads, context mirrors and frames that predate the SDK's event watcher. */
import assert from 'node:assert/strict'
import { once } from 'node:events'
import fs from 'node:fs'
import http from 'node:http'
import os from 'node:os'
import path from 'node:path'
import test from 'node:test'
import { browserVersion, launchBrowser } from './browser.mjs'
import { openClaw } from './claw-loader.mjs'

test('lifecycle events preserve existing and new frame identity', {
  skip: !process.env.NEO_E2E_BROWSER,
  timeout: 60_000,
}, async (t) => {
  const output = fs.mkdtempSync(
    path.join(
      process.env.NEO_NODE_OUTPUT_DIR || os.tmpdir(),
      'page-lifecycle-',
    ),
  )
  t.diagnostic(`Evidence: ${output}`)
  const server = http.createServer((_req, res) => {
    res.setHeader('content-type', 'text/html')
    res.end('<title>Lifecycle</title><h1>Ready</h1>')
  })
  server.listen(0, '127.0.0.1')
  await once(server, 'listening')
  const url = `http://127.0.0.1:${server.address().port}/`
  const executable = process.env.NEO_E2E_BROWSER
  let browser
  let sdk
  const passed = []
  const record = (name) => {
    passed.push(name)
    fs.writeFileSync(
      path.join(output, 'proof.json'),
      JSON.stringify({ passed }, null, 2),
    )
  }
  const open = () =>
    openClaw(browser.port, {
      timeoutMs: 45_000,
      stateDir: path.join(output, `sdk-${passed.length}`),
      browserosVersion: browserVersion(executable),
    })
  try {
    browser = await launchBrowser(executable, path.join(output, 'worker-0'), {
      launcherPid: process.pid,
    })
    sdk = await open()
    const adopted = sdk.context.pages()[0]
    assert(adopted)
    await adopted.goto(url)
    await adopted.setContent(
      '<iframe name="existing" src="about:blank"></iframe>',
    )
    await sdk.finish()
    sdk = await open()
    let page = sdk.context.pages().find((page) => page.url() === url)
    assert(page, 'an adopted page survives the earlier SDK run')
    await page.waitForLoadState()
    const existing = page.frames()[1]
    assert(existing)
    const detached = page.waitForEvent('framedetached', { timeout: 3000 })
    await page.evaluate(() => document.querySelector('iframe').remove())
    assert.equal(await detached, existing)
    assert(existing.isDetached())
    assert.equal(existing.parentFrame(), page.mainFrame())
    record(
      'a frame created before the watcher emits its removal with the original Frame',
    )

    await sdk.finish()
    sdk = await open()
    page = sdk.context.pages().find((candidate) => candidate.url() === url)
    assert(page)
    // Await the public operation's setup so the external CDP action tests observation on a
    // tab that this SDK instance has never used, without relying on a wall-clock sleep.
    const setup = JSON.parse(
      await sdk.native.invoke(
        'events.listen',
        JSON.stringify({
          call: { api: 'browserContext.on', timeoutMs: 3000 },
          page: null,
          event: 'pageload',
          on: true,
          since: Date.now(),
        }),
        'lifecycle setup',
      ),
    )
    assert(!setup.err, JSON.stringify(setup.err))
    const targets = await (
      await fetch(`http://127.0.0.1:${browser.port}/json/list`)
    ).json()
    const target = targets.find((target) => target.url === url)
    assert(target)
    const socket = new WebSocket(target.webSocketDebuggerUrl)
    try {
      await once(socket, 'open')
      const loaded = sdk.context.waitForEvent('pageload', { timeout: 3000 })
      socket.send(
        JSON.stringify({
          id: 1,
          method: 'Page.navigate',
          params: { url: `${url}untouched` },
        }),
      )
      const eventPage = await loaded
      assert.equal(eventPage, page)
      assert.equal(eventPage.url(), `${url}untouched`)
      record(
        'context observes an untouched tab and supplies its current load URL',
      )
    } finally {
      socket.close()
    }

    const frameAttached = page.waitForEvent('frameattached', { timeout: 3000 })
    const contextAttached = sdk.context.waitForEvent('frameattached', {
      timeout: 3000,
    })
    await page.evaluate(() => {
      const frame = document.createElement('iframe')
      frame.src = 'about:blank'
      document.body.append(frame)
    })
    const frame = await frameAttached
    assert.equal(await contextAttached, frame)
    assert.equal(frame.parentFrame(), page.mainFrame())
    record('page and context receive the same attached Frame')

    const navigated = page.waitForEvent('framenavigated', {
      timeout: 3000,
      predicate: (candidate) => candidate.url().endsWith('#target'),
    })
    await page.evaluate(() => {
      history.pushState({}, '', '#ignored')
      history.pushState({}, '', '#target')
    })
    assert.equal(await navigated, page.mainFrame())
    record('navigation predicates see transient same-document URLs')

    const loaded = page.waitForEvent('load', { timeout: 3000 })
    const contextLoaded = sdk.context.waitForEvent('pageload', {
      timeout: 3000,
    })
    const dom = page.waitForEvent('domcontentloaded', { timeout: 3000 })
    await page.reload()
    assert.equal(await loaded, page)
    assert.equal(await contextLoaded, page)
    assert.equal(await dom, page)
    record('page load and DOMContentLoaded and context pageload carry Page')

    await assert.rejects(
      page.waitForEvent('load', { timeout: 50 }),
      (error) => error.name === 'TimeoutError',
    )
    record('an event wait times out without an event')
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
