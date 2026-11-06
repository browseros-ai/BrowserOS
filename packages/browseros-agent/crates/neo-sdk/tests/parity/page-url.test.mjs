// Neo against real Playwright on the same BrowserOS tab: watch-url.js runs in both at once
// while this test changes the tab's URL over CDP and then closes it. Both results must be
// identical, and equal to what the design measured for Playwright.
//
// Runs under Node, not Bun: Playwright's connectOverCDP hangs on Bun's WebSocket. Gated on
// NEO_E2E_BROWSER, the path of a BrowserOS neo binary.

import assert from 'node:assert/strict'
import { execFileSync, spawn } from 'node:child_process'
import { once } from 'node:events'
import { mkdtempSync, readFileSync, rmSync } from 'node:fs'
import http from 'node:http'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'
import { test } from 'node:test'
import { fileURLToPath } from 'node:url'
import { chromium } from 'playwright-core'

const browserBinary = process.env.NEO_E2E_BROWSER
const here = dirname(fileURLToPath(import.meta.url))
const workspaceRoot = join(here, '../../../..')
const scriptPath = join(here, '../scripts/watch-url.js')
const STEP_MS = 300
const SLOW_RESPONSE_MS = 2500
const expected = {
  paths: ['/a', '/a#done', '/pushed', '/slow'],
  closed: true,
  lastPath: '/slow',
}

test('watch-url.js returns the same value through Neo and through Playwright', {
  skip: browserBinary ? false : 'NEO_E2E_BROWSER is unset',
  timeout: 600_000,
}, async (t) => {
  const neoRun = buildNeoRun()
  const cleanups = []
  try {
    const origin = await serveFixture(cleanups)
    const browser = await launchBrowser(cleanups, browserBinary)
    const cdp = await connectCdp(cleanups, browser.port)
    const { targetId } = await cdp.send('Target.createTarget', {
      url: `${origin}/a`,
    })
    const { sessionId: driver } = await cdp.send('Target.attachToTarget', {
      targetId,
      flatten: true,
    })
    await waitForUrl(cdp, driver, `${origin}/a`)

    const playwright = await chromium.connectOverCDP(
      `http://127.0.0.1:${browser.port}`,
    )
    cleanups.push(() => playwright.close())
    const [context] = playwright.contexts()
    const page = await findPage(context, targetId)

    const neo = startNeo(neoRun, browser.port, targetId)
    await neo.connected
    const consoleLog = t.mock.method(console, 'log', () => {})
    const AsyncFunction = (async () => {}).constructor
    const source = readFileSync(scriptPath, 'utf8')
    const viaPlaywright = new AsyncFunction('page', 'context', source)(
      page,
      context,
    )

    await driveTab(cdp, driver, origin, targetId)
    const [neoOutcome, playwrightValue] = await Promise.all([
      neo.outcome,
      viaPlaywright,
    ])
    const playwrightLogs = consoleLog.mock.calls.map((call) =>
      call.arguments.join(' '),
    )
    consoleLog.mock.restore()

    assert.equal(neoOutcome.status, 'ok', JSON.stringify(neoOutcome))
    assert.deepEqual(playwrightValue, expected, 'Playwright')
    assert.deepEqual(neoOutcome.value, playwrightValue, 'Neo vs Playwright')
    assert.deepEqual(neoOutcome.logs, playwrightLogs, 'console.log lines')
  } finally {
    for (const cleanup of cleanups.reverse()) await cleanup()
  }
})

/** Builds the `neo_run` example and returns the path cargo reports for it. */
function buildNeoRun() {
  const output = execFileSync(
    'cargo',
    [
      'build',
      '-p',
      'neo-runtime',
      '--example',
      'neo_run',
      '--message-format=json-render-diagnostics',
    ],
    { cwd: workspaceRoot, encoding: 'utf8', maxBuffer: 64 * 1024 * 1024 },
  )
  const artifact = output
    .split('\n')
    .filter((line) => line.startsWith('{'))
    .map((line) => JSON.parse(line))
    .find(
      (message) =>
        message.reason === 'compiler-artifact' &&
        message.target.name === 'neo_run' &&
        message.executable,
    )
  assert.ok(artifact, 'cargo reported no neo_run executable')
  return artifact.executable
}

/** Runs watch-url.js through Neo. `connected` settles once neo_run has a CDP connection. */
function startNeo(neoRun, port, targetId) {
  const child = spawn(neoRun, [
    '--cdp-port',
    String(port),
    '--page',
    targetId,
    '--timeout-ms',
    '60000',
    scriptPath,
  ])
  let stdout = ''
  let stderr = ''
  child.stdout.on('data', (chunk) => {
    stdout += chunk
  })
  const exited = once(child, 'exit')
  const connected = new Promise((resolve, reject) => {
    child.stderr.on('data', (chunk) => {
      stderr += chunk
      if (stderr.includes('connected')) resolve()
    })
    exited.then(() => reject(new Error(`neo_run exited early: ${stderr}`)))
  })
  const outcome = exited.then(([code]) => {
    assert.notEqual(code, 2, `neo_run failed to run: ${stderr}`)
    return JSON.parse(stdout)
  })
  return { connected, outcome }
}

/** The same steps as crates/neo-runtime/tests/sync_getters.rs, STEP_MS apart. */
async function driveTab(cdp, session, origin, targetId) {
  const evaluate = async (expression) => {
    const reply = await cdp.send('Runtime.evaluate', { expression }, session)
    assert.equal(reply.exceptionDetails, undefined, `${expression} threw`)
  }
  await sleep(STEP_MS)
  await evaluate("location.hash = 'done'")
  await sleep(STEP_MS)
  await evaluate("history.pushState({}, '', '/pushed')")
  await sleep(STEP_MS)
  await cdp.send('Page.navigate', { url: `${origin}/slow` }, session)
  await waitForUrl(cdp, session, `${origin}/slow`)
  await sleep(STEP_MS)
  await cdp.send('Target.closeTarget', { targetId })
}

/**
 * Waits until the tab's committed URL is `url`. A read can fail for a few milliseconds while
 * a cross-document navigation commits ("Not attached to an active page"), so failures retry.
 */
async function waitForUrl(cdp, session, url) {
  const deadline = Date.now() + 15_000
  let current
  while (current !== url) {
    assert.ok(Date.now() < deadline, `the tab never committed ${url}`)
    current = await cdp
      .send('Page.getNavigationHistory', {}, session)
      .then(({ currentIndex, entries }) => entries[currentIndex].url)
      .catch((error) => error.message)
    if (current !== url) await sleep(20)
  }
}

/** Finds Playwright's page for a CDP target id. */
async function findPage(context, targetId) {
  const deadline = Date.now() + 10_000
  while (Date.now() < deadline) {
    for (const page of context.pages()) {
      const session = await context.newCDPSession(page)
      const { targetInfo } = await session.send('Target.getTargetInfo')
      await session.detach()
      if (targetInfo.targetId === targetId) return page
    }
    await sleep(50)
  }
  assert.fail(`Playwright never saw target ${targetId}`)
}

/** Serves an HTML page for any path; `/slow` answers after SLOW_RESPONSE_MS. */
async function serveFixture(cleanups) {
  const server = http.createServer(async (request, response) => {
    if (request.url === '/slow') await sleep(SLOW_RESPONSE_MS)
    response.setHeader('content-type', 'text/html')
    response.end(`<title>${request.url}</title><h1>${request.url}</h1>`)
  })
  server.listen(0, '127.0.0.1')
  await once(server, 'listening')
  cleanups.push(() => {
    server.closeAllConnections()
    server.close()
  })
  return `http://127.0.0.1:${server.address().port}`
}

/**
 * Launches a headless BrowserOS neo on a throwaway profile. Its cleanup ends the browser,
 * then checks nothing started from that profile is still running.
 */
async function launchBrowser(cleanups, binary) {
  const profile = mkdtempSync(join(tmpdir(), 'neo-parity-'))
  const child = spawn(
    binary,
    [
      '--headless=new',
      // Otherwise the browser starts a browseros-claw-server sidecar that outlives it.
      '--disable-browseros-server',
      // Port 0 lets the browser pick a free port and write it to DevToolsActivePort.
      '--remote-debugging-port=0',
      `--user-data-dir=${profile}`,
      '--no-first-run',
      'about:blank',
    ],
    { stdio: 'ignore' },
  )
  cleanups.push(async () => {
    const helpers = processIds(['-P', String(child.pid)])
    const exited = once(child, 'exit')
    child.kill('SIGTERM')
    const forced = setTimeout(() => child.kill('SIGKILL'), 5_000)
    await exited
    clearTimeout(forced)
    // Helpers notice their parent is gone on their own, usually within a second.
    const deadline = Date.now() + 5_000
    let lingering = [...helpers, ...processIds(['-f', profile])]
    while (lingering.length > 0 && Date.now() < deadline) {
      await sleep(100)
      lingering = lingering.filter(isAlive)
    }
    for (const pid of lingering) process.kill(Number(pid), 'SIGKILL')
    rmSync(profile, { recursive: true, force: true })
    assert.deepEqual(lingering, [], 'browser processes outlived the browser')
  })

  const deadline = Date.now() + 20_000
  for (;;) {
    try {
      const port = Number(
        readFileSync(join(profile, 'DevToolsActivePort'), 'utf8').split(
          '\n',
        )[0],
      )
      if (port > 0) return { port }
    } catch {}
    assert.ok(Date.now() < deadline, 'the browser never opened its CDP port')
    await sleep(50)
  }
}

function processIds(pgrepArgs) {
  try {
    return execFileSync('pgrep', pgrepArgs, { encoding: 'utf8' })
      .split('\n')
      .filter(Boolean)
  } catch {
    return []
  }
}

function isAlive(pid) {
  try {
    process.kill(Number(pid), 0)
    return true
  } catch {
    return false
  }
}

/** A minimal CDP client on the browser's WebSocket, separate from Playwright's. */
async function connectCdp(cleanups, port) {
  const version = await fetch(`http://127.0.0.1:${port}/json/version`)
  const { webSocketDebuggerUrl } = await version.json()
  const socket = new WebSocket(webSocketDebuggerUrl)
  await once(socket, 'open')
  cleanups.push(() => socket.close())
  const pending = new Map()
  let nextId = 1
  socket.addEventListener('message', ({ data }) => {
    const message = JSON.parse(data)
    const call = pending.get(message.id)
    if (!call) return
    pending.delete(message.id)
    if (message.error) {
      call.reject(new Error(`${call.method}: ${message.error.message}`))
    } else {
      call.resolve(message.result)
    }
  })
  return {
    send(method, params = {}, sessionId) {
      return new Promise((resolve, reject) => {
        const id = nextId++
        pending.set(id, { method, resolve, reject })
        socket.send(JSON.stringify({ id, method, params, sessionId }))
      })
    },
  }
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms))
}
