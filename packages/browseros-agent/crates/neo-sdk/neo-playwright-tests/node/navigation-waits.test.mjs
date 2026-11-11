// biome-ignore-all lint/suspicious/noUndeclaredEnvVars: This standalone Node harness is not a Turbo task.
/** Real SDK navigation seams: one deadline, retained events, adopted OOPIFs and document identity. */
import assert from 'node:assert/strict'
import { once } from 'node:events'
import fs from 'node:fs'
import http from 'node:http'
import os from 'node:os'
import path from 'node:path'
import test from 'node:test'
import { browserVersion, launchBrowser } from './browser.mjs'
import { openClaw } from './claw-loader.mjs'

test('navigation waits retain events, document identity and their deadline', {
  skip: !process.env.NEO_E2E_BROWSER,
  timeout: 120_000,
}, async (t) => {
  const output = fs.mkdtempSync(
    path.join(
      process.env.NEO_NODE_OUTPUT_DIR || os.tmpdir(),
      'navigation-waits-',
    ),
  )
  fs.mkdirSync(output, { recursive: true })
  const server = http.createServer((req, res) => {
    res.setHeader('content-type', 'text/html')
    if (req.url === '/rapid-a') {
      res.statusCode = 201
      res.end('<script>location="/rapid-b"</script>')
      return
    }
    if (req.url === '/rapid-b') {
      res.statusCode = 503
      res.end('<h1>Later response</h1>')
      return
    }
    res.end('<title>Navigation regression</title><h1>Ready</h1>')
  })
  server.listen(0, '127.0.0.1')
  await once(server, 'listening')
  const url = `http://127.0.0.1:${server.address().port}/start`
  const executable = process.env.NEO_E2E_BROWSER
  let browser
  let sdk
  const passed = []
  t.diagnostic(`Evidence: ${output}`)
  const record = (label) => {
    passed.push(label)
    fs.writeFileSync(
      path.join(output, 'proof.json'),
      JSON.stringify({ passed }, null, 2),
    )
  }
  try {
    browser = await launchBrowser(executable, path.join(output, 'worker-0'), {
      launcherPid: process.pid,
    })
    sdk = await openClaw(browser.port, {
      timeoutMs: 60000,
      stateDir: path.join(output, 'sdk-state'),
      browserosVersion: browserVersion(executable),
    })
    const page = await sdk.context.newPage()
    await page.goto(url)
    await assert.rejects(
      page.waitForNavigation({ timeout: 100 }),
      (e) =>
        e.name === 'TimeoutError' && /Timeout 100ms exceeded/.test(e.message),
    )
    record('existing URL does not satisfy navigation wait')
    const [response] = await Promise.all([
      page.waitForNavigation({ timeout: 3000 }),
      page.reload(),
    ])
    assert.equal(response.url(), url)
    assert.equal(response.status(), 200)
    record('same URL reload returns its response')
    const [history] = await Promise.all([
      page.waitForNavigation({
        url: (u) => u.pathname === '/target',
        timeout: 3000,
      }),
      page.evaluate(() => {
        history.pushState({}, '', '/first')
        history.pushState({}, '', '/target')
        history.pushState({}, '', '/last')
      }),
    ])
    assert.equal(history, null)
    assert.match(page.url(), /\/last$/)
    await page.title()
    assert.match(page.url(), /\/last$/)
    record('predicate sees transient URL and later reads retain its fresh URL')
    const [timed] = await Promise.all([
      page
        .waitForNavigation({ url: () => new Promise(() => {}), timeout: 100 })
        .catch((e) => e),
      page.evaluate(() => history.pushState({}, '', '/pending')),
    ])
    assert.equal(timed.name, 'TimeoutError')
    assert.match(timed.message, /Timeout 100ms exceeded/)
    record('pending predicate shares the navigation deadline')
    await page.setContent('<iframe src="about:blank"></iframe>')
    const frame = page.frames()[1]
    const [detached] = await Promise.all([
      frame.waitForNavigation({ timeout: 3000 }).catch((e) => e),
      page.evaluate(() => document.querySelector('iframe').remove()),
    ])
    assert.match(detached.message, /frame was detached/i)
    record('detached child rejects navigation wait')
    const adopted = sdk.context
      .pages()
      .find((candidate) => candidate !== page && !candidate.isClosed())
    assert(adopted, 'launch tab must exist before SDK ownership')
    await adopted.goto(url)
    await adopted.setContent(`<iframe src="${url}/child"></iframe>`)
    await adopted.frames()[1].waitForLoadState()
    const child = adopted.frames()[1]
    const cross = `${url.replace('127.0.0.1', 'localhost')}/cross`
    const [childResponse] = await Promise.all([
      child.waitForNavigation({ timeout: 5000 }),
      adopted.evaluate(
        (next) => (document.querySelector('iframe').src = next),
        cross,
      ),
    ])
    assert.equal(childResponse.frame(), child)
    assert.equal(childResponse.url(), cross)
    record(
      'adopted tab child crossing renderers keeps its navigation and response',
    )
    const selected = page.waitForNavigation({
      url: '**/rapid-a',
      timeout: 5000,
    })
    const redirected = page.goto(new URL('/rapid-a', url).href).catch(() => {})
    const selectedResponse = await selected
    await redirected
    assert(
      selectedResponse === null ||
        (selectedResponse.url().endsWith('/rapid-a') &&
          selectedResponse.status() === 201),
    )
    record(
      'rapid replacement never lends a later document response to an earlier match',
    )
    await page.close()
    // A fresh host adopts the existing OOPIF on a new CDP session. Listing its Frame must
    // enable navigation reporting before a same-renderer fragment change can race a waiter.
    await sdk.finish()
    sdk = await openClaw(browser.port, {
      timeoutMs: 60000,
      stateDir: path.join(output, 'adopted-sdk-state'),
      browserosVersion: browserVersion(executable),
    })
    const existing = sdk.context
      .pages()
      .find((candidate) => candidate.url() === url)
    assert(existing, 'the adopted launch tab survives the first host')
    await existing.waitForLoadState()
    await existing.evaluate(() => 1)
    const existingChild = existing.frames()[1]
    const [fragmentResponse] = await Promise.all([
      existingChild.waitForNavigation({ timeout: 3000 }),
      existing.evaluate(
        (next) => (document.querySelector('iframe').src = next),
        `${cross}#adopted`,
      ),
    ])
    assert.equal(fragmentResponse, null)
    assert.equal(existingChild.url(), `${cross}#adopted`)
    record('adopted OOPIF reports same-renderer navigation before wait setup')
    await existing.close()
    fs.writeFileSync(
      path.join(output, 'proof.json'),
      JSON.stringify({ passed }, null, 2),
    )
  } finally {
    // Finishing the run and stopping the browser own different resources: either failure must
    // still release the other, and the guardian verifies only this test's private process group.
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
