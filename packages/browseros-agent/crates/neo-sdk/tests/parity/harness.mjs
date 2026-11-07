// What every parity test shares: one headless BrowserOS neo per test, a fixture server, a raw
// CDP client for driving tabs, real Playwright over connectOverCDP, and Neo through the neo_run
// example. Each test pushes its cleanups and runs them in reverse; the browser's cleanup closes
// it as Playwright does and fails the test if any of its processes outlives it.
//
// Runs under Node, not Bun: Playwright's connectOverCDP hangs on Bun's WebSocket. Gated on
// NEO_E2E_BROWSER, the path of a BrowserOS neo binary. The neo_run build honours
// CARGO_TARGET_DIR and CARGO_BUILD_BUILD_DIR from the environment.

import assert from 'node:assert/strict'
import { execFileSync, spawn } from 'node:child_process'
import { once } from 'node:events'
import {
  mkdirSync,
  mkdtempSync,
  readFileSync,
  realpathSync,
  rmSync,
  writeFileSync,
} from 'node:fs'
import http from 'node:http'
import { tmpdir } from 'node:os'
import { dirname, join, sep } from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'

// Playwright dims the call log in error messages whenever the environment suggests colour
// (`COLORTERM`, `TERM=xterm-*`, inherited from the terminal even without a TTY), and reads the
// setting when it loads. The goldens were recorded with colours off, so this must run before
// playwright-core is imported, and tests take `chromium` from here, never from playwright-core.
process.env.FORCE_COLOR = '0'
/**
 * The Playwright every parity test compares against: `playwright-core` from npm (this package's
 * devDependency), or a build of Playwright's source at the vendored commit when
 * `NEO_PARITY_PLAYWRIGHT` names its `packages/playwright-core` directory (`npm ci && npm run build`
 * in a checkout of its own). The two are different revisions; tests report `playwrightVersion`
 * so every result names the one it ran against.
 */
const playwrightDir = process.env.NEO_PARITY_PLAYWRIGHT
// Two literal imports, so dependency checks still see `playwright-core` used.
export const { chromium, request } = playwrightDir
  ? await import(pathToFileURL(join(playwrightDir, 'index.mjs')).href)
  : await import('playwright-core')
export const playwrightVersion = JSON.parse(
  readFileSync(
    playwrightDir
      ? join(playwrightDir, 'package.json')
      : fileURLToPath(import.meta.resolve('playwright-core/package.json')),
    'utf8',
  ),
).version

export const browserBinary = process.env.NEO_E2E_BROWSER
/**
 * How long `closeBrowser` waits for the browser to exit after `Browser.close` before it SIGKILLs
 * the browser's process group: Playwright's `closeOrKill` timeout (`DEFAULT_PLAYWRIGHT_TIMEOUT`).
 */
const CLOSE_TIMEOUT_MS = 30_000
/**
 * How long a helper may outlive a browser that closed gracefully before `closeBrowser` reports
 * it as a survivor and SIGKILLs it. neo-runtime's `tests/support/browser.rs` uses the same value.
 */
const HELPER_GRACE_MS = 5_000
export const skip = browserBinary ? false : 'NEO_E2E_BROWSER is unset'
const workspaceRoot = join(
  dirname(fileURLToPath(import.meta.url)),
  '../../../..',
)

/** Builds the `neo_run` example and returns the path cargo reports for it. */
export function buildNeoRun() {
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

/**
 * Runs the script file at `scriptPath` through Neo against `targetId`. `connected` settles once
 * neo_run has a CDP connection; `outcome` is the `RunOutcome` it prints. `extraArgs` go before
 * the script, such as `['--artifacts', dir]`.
 */
export function startNeo(neoRun, port, targetId, scriptPath, extraArgs = []) {
  const child = spawn(neoRun, [
    '--cdp-port',
    String(port),
    '--page',
    targetId,
    '--timeout-ms',
    '60000',
    ...extraArgs,
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

/**
 * A rejection's kind: the error's name, with `:strict` for a strict mode violation (a plain
 * `Error` in Playwright), never its wording. Shared inside each script, so both engines report
 * their rejections in the same way and a `TypeError` never passes for an `Error`.
 */
export function errorCategory(error) {
  const strict = /strict mode violation/.test(error.message)
  return strict ? `${error.name}:strict` : error.name
}

/**
 * Runs one method's public script on fresh pages and compares its result and browser effect.
 * `effects({ playwright, neo })`, for a deliberate difference, asserts both effects itself in
 * place of the equality check; `values({ playwright, neo })` does the same for the values.
 * `isolate` runs Neo in a browser of its own, its effect read over raw CDP: for what Playwright
 * installs context-wide in the shared browser, its exposed bindings' `__playwright__binding__`
 * above all, which would answer Neo's page too.
 */
export async function compareMethod(
  t,
  { html, source, probe, effects, values, isolate = false, slowMs = 0 },
) {
  t.diagnostic(
    `Playwright ${playwrightVersion}; ${playwrightDir ?? 'npm playwright-core'}`,
  )
  const neoRun = buildNeoRun()
  const scriptSource = `const errorCategory = ${errorCategory.toString()};\n${source}`
  const cleanups = []
  try {
    const origin = await serveFixture(cleanups, {
      pages: { '/method.html': html },
      slowMs,
    })
    const browser = await launchBrowser(cleanups, browserBinary)
    const playwright = await chromium.connectOverCDP(
      `http://127.0.0.1:${browser.port}`,
    )
    cleanups.push(() => playwright.close())
    const [context] = playwright.contexts()
    const scripts = mkdtempSync(join(tmpdir(), 'neo-method-parity-'))
    cleanups.push(() => rmSync(scripts, { recursive: true, force: true }))
    const reference = await context.newPage()
    await reference.goto(`${origin}/method.html`)
    const AsyncFunction = (async () => {}).constructor
    const value = await new AsyncFunction('page', scriptSource)(reference)
    const effect = await reference.evaluate(probe)
    await reference.close()

    const script = join(scripts, 'method.js')
    writeFileSync(script, scriptSource)
    let outcome
    let actualEffect
    if (isolate) {
      const own = await launchBrowser(cleanups, browserBinary)
      const cdp = await connectCdp(cleanups, own.port)
      const { targetId, sessionId } = await openTab(
        cdp,
        `${origin}/method.html`,
      )
      const neo = startNeo(neoRun, own.port, targetId, script)
      await neo.connected
      outcome = await neo.outcome
      const probed = await cdp.send(
        'Runtime.evaluate',
        { expression: probe, returnByValue: true, awaitPromise: true },
        sessionId,
      )
      actualEffect = probed.result.value
    } else {
      const page = await context.newPage()
      await page.goto(`${origin}/method.html`)
      const session = await context.newCDPSession(page)
      const { targetInfo } = await session.send('Target.getTargetInfo')
      await session.detach()
      const neo = startNeo(neoRun, browser.port, targetInfo.targetId, script)
      await neo.connected
      outcome = await neo.outcome
      actualEffect = await page.evaluate(probe)
      await page.close()
    }
    assert.equal(outcome.status, 'ok', JSON.stringify(outcome))
    if (values) values({ playwright: value, neo: outcome.value })
    else
      assert.deepEqual(outcome.value, value, 'public return values and types')
    if (effects) effects({ playwright: effect, neo: actualEffect })
    else assert.deepEqual(actualEffect, effect, 'browser effect')
    return outcome
  } finally {
    await runCleanups(cleanups)
  }
}

/**
 * Waits until the tab's committed URL is `url`. A read can fail for a few milliseconds while
 * a cross-document navigation commits ("Not attached to an active page"), so failures retry.
 */
export async function waitForUrl(cdp, session, url) {
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
export async function findPage(context, targetId) {
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

/**
 * Opens a tab on `url` with the test's own CDP client, as a tab the user had open before a run,
 * and returns its target id and an attached session once the tab has committed `url`.
 */
export async function openTab(cdp, url) {
  const { targetId } = await cdp.send('Target.createTarget', { url })
  const { sessionId } = await cdp.send('Target.attachToTarget', {
    targetId,
    flatten: true,
  })
  await waitForUrl(cdp, sessionId, url)
  return { targetId, sessionId }
}

/**
 * Runs an agent script's source on the tab `targetId` through Playwright, with Playwright's
 * `page` and `context` in scope as neo_run puts Neo's. Settles with `{ value }` or
 * `{ error: { name, message } }`.
 */
export async function runInPlaywright(context, targetId, source) {
  const page = await findPage(context, targetId)
  const AsyncFunction = (async () => {}).constructor
  return await new AsyncFunction('page', 'context', source)(page, context).then(
    (value) => ({ value }),
    (error) => ({ error: { name: error.name, message: error.message } }),
  )
}

/**
 * Runs the script file at `scriptPath` on the tab `targetId` through Neo. Settles as
 * `runInPlaywright` does, plus the run's whole `outcome`.
 */
export async function runInNeo(neoRun, port, targetId, scriptPath) {
  const neo = startNeo(neoRun, port, targetId, scriptPath)
  await neo.connected
  const outcome = await neo.outcome
  return outcome.status === 'ok'
    ? { value: outcome.value, outcome }
    : { error: outcome.error, outcome }
}

/** Calls `check` every 50 ms until it returns true, and fails once `timeoutMs` has passed. */
export async function eventually(check, what, timeoutMs = 15_000) {
  const deadline = Date.now() + timeoutMs
  while (!(await check())) {
    assert.ok(Date.now() < deadline, `timed out waiting until ${what}`)
    await sleep(50)
  }
}

/**
 * Serves `pages[path]` for a path it names (with or without a query, so one frame page serves
 * every `?who=`), and a small page titled with the path otherwise. `/slow` answers after
 * `slowMs`, so a navigation to it stays in flight before it commits. It listens on 127.0.0.1;
 * the frame tests also load it as `localhost`, a different site, which BrowserOS neo reaches on
 * the same address.
 *
 * A page is its HTML, or `{ body, headers, status }` for a response with headers of its own (an
 * attachment), or a function `(request, response)` that answers by itself (a body sent slowly).
 */
export async function serveFixture(cleanups, { pages = {}, slowMs = 0 } = {}) {
  const server = http.createServer(async (request, response) => {
    if (request.url === '/slow') await sleep(slowMs)
    const page = pages[request.url] ??
      pages[request.url.split('?')[0]] ?? {
        body: `<title>${request.url}</title><h1>${request.url}</h1>`,
      }
    if (typeof page === 'function') return page(request, response)
    const {
      body,
      headers = {},
      status = 200,
    } = typeof page === 'string' ? { body: page } : page
    response.statusCode = status
    response.setHeader('content-type', 'text/html; charset=utf-8')
    for (const [name, value] of Object.entries(headers)) {
      response.setHeader(name, value)
    }
    response.end(body)
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
 * A serveFixture page answering with the file at `?path=`, such as a download's `path()`, so a
 * script reads the file back through its page: no engine's script reads files. 404 when there is
 * none, and 403 for a file outside the temporary directory, which holds Playwright's artifacts
 * and every test profile: a download saved anywhere else, as in the user's ~/Downloads, reads as
 * refused.
 */
export function fileContent(request, response) {
  const file = new URL(request.url, 'http://fixture').searchParams.get('path')
  let status = 404
  let body = ''
  try {
    const real = realpathSync(file)
    status = real.startsWith(realpathSync(tmpdir()) + sep) ? 200 : 403
    if (status === 200) body = readFileSync(real)
  } catch {}
  response.statusCode = status
  response.setHeader('content-type', 'text/plain; charset=utf-8')
  response.end(body)
}

/**
 * Launches a headless BrowserOS neo on a throwaway profile, leading a process group of its own.
 * Every helper it starts (GPU, utility, renderers) joins that group (`ps -o pid,pgid` on
 * 155.0.8318.26), so one signal to the group reaches them all. Its cleanup is `closeBrowser`.
 * `args` go before the first URL, such as Playwright's own `--disable-back-forward-cache`.
 *
 * The profile's download folder is its own `downloads` directory, returned as `downloads`: a
 * download the browser saves itself (Neo's `behavior: 'default'`) goes to the profile's default
 * folder, which a fresh profile puts at the user's ~/Downloads. "Ask where to save each file" is
 * off, as a headless browser cancels a download that asks.
 */
export async function launchBrowser(cleanups, binary, args = []) {
  const profile = mkdtempSync(join(tmpdir(), 'neo-parity-'))
  const downloads = join(profile, 'downloads')
  mkdirSync(downloads)
  mkdirSync(join(profile, 'Default'))
  writeFileSync(
    join(profile, 'Default', 'Preferences'),
    JSON.stringify({
      download: {
        default_directory: downloads,
        prompt_for_download: false,
        directory_upgrade: true,
      },
      savefile: { default_directory: downloads },
    }),
  )
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
      ...args,
      'about:blank',
    ],
    // `detached` makes it a group leader, as Playwright's launcher does, so one SIGKILL to the
    // group reaches every helper.
    { stdio: 'ignore', detached: true },
  )
  // Every group signal below goes to `-pid`. Were the browser in the test runner's group, that
  // would kill `node --test` with it, so confirm it leads its own before anything relies on it.
  const pgid = execFileSync('ps', ['-o', 'pgid=', '-p', String(child.pid)], {
    encoding: 'utf8',
  }).trim()
  if (pgid !== String(child.pid)) {
    child.kill('SIGKILL')
    rmSync(profile, { recursive: true, force: true })
    assert.fail(
      `the browser (pid ${child.pid}) is in process group ${pgid}, not its own; refusing group signals`,
    )
  }
  const exited = once(child, 'exit')
  const browser = { child, exited, profile, endpoint: undefined }
  cleanups.push(() => closeBrowser(browser))

  const deadline = Date.now() + 20_000
  for (;;) {
    try {
      const [port, path] = readFileSync(
        join(profile, 'DevToolsActivePort'),
        'utf8',
      ).split('\n')
      if (Number(port) > 0 && path) {
        browser.endpoint = `ws://127.0.0.1:${port}${path}`
        return { port: Number(port), downloads }
      }
    } catch {}
    assert.ok(Date.now() < deadline, 'the browser never opened its CDP port')
    await sleep(50)
  }
}

/**
 * Closes the browser as Playwright does (packages/utils/processLauncher.ts `gracefullyClose`
 * and `killProcess`, server/browserType.ts `closeOrKill`, at 26137face): `Browser.close` over
 * CDP, then wait for the process to exit, and SIGKILL the whole group if it has not within
 * CLOSE_TIMEOUT_MS. Then it waits out the helpers and SIGKILLs the group again, so nothing
 * outlives the test.
 *
 * Every shutdown prints its timings on stderr as `browser shutdown {…}`: the browser's exit
 * after `Browser.close` (`forced` when it needed SIGKILL), each helper's exit after the
 * browser's, and the `survivors` still running after HELPER_GRACE_MS. A survivor is reported,
 * not failed: it is Chromium's shutdown, which Neo has no part in (numbers in neo-runtime's
 * `tests/support/browser.rs`, `Browser::close`). A process still running after the group's
 * SIGKILL fails the test.
 */
async function closeBrowser({ child, exited, profile, endpoint }) {
  const helpers = groupProcesses(child.pid, profile)
  const stopping = Date.now()
  let forced = false
  const force = setTimeout(() => {
    forced = true
    killGroup(child.pid)
  }, CLOSE_TIMEOUT_MS)
  // Not awaited before the exit: a socket that never opens must not hold the shutdown up.
  const closing = sendBrowserClose(endpoint).catch(() => {
    forced = true
    killGroup(child.pid)
  })
  await exited
  clearTimeout(force)
  closing.then((socket) => socket?.close())
  const browserExitMs = Date.now() - stopping

  const browserGone = Date.now()
  const exits = []
  let alive = livePids(child.pid, profile)
  for (;;) {
    for (const [pid, type] of helpers) {
      if (alive.includes(pid)) continue
      exits.push({ pid, type, exitMs: Date.now() - browserGone })
      helpers.delete(pid)
    }
    if (alive.length === 0 || Date.now() - browserGone >= HELPER_GRACE_MS) break
    await sleep(20)
    alive = livePids(child.pid, profile)
  }
  const survivors = alive.map((pid) => ({
    pid,
    type: helpers.get(pid) ?? processType(pid),
  }))
  const left = await killAll(child.pid, profile)
  rmSync(profile, { recursive: true, force: true })
  process.stderr.write(
    `browser shutdown ${JSON.stringify({ browserExitMs, forced, helpers: exits, survivors })}\n`,
  )
  assert.deepEqual(
    left,
    [],
    'browser processes outlived the test even after SIGKILL',
  )
}

/** Sends `Browser.close` on a socket of its own; the browser closes it as it exits. */
async function sendBrowserClose(endpoint) {
  assert.ok(endpoint, 'the browser never published its CDP endpoint')
  const socket = new WebSocket(endpoint)
  await new Promise((resolve, reject) => {
    socket.addEventListener('open', resolve, { once: true })
    socket.addEventListener('error', reject, { once: true })
  })
  socket.send(JSON.stringify({ id: 1, method: 'Browser.close' }))
  return socket
}

/**
 * SIGKILL to the browser's whole group, as Playwright's `killProcess` sends it. `launchBrowser`
 * checked that the browser leads that group. Once the browser has exited, the group's id stays
 * reserved only while a member is alive (POSIX never reuses the id of a live group), so the
 * signal goes only to a group that still has members: an empty group's id may be someone else's.
 */
function killGroup(pid) {
  if (processIds(['-g', String(pid)]).length === 0) return
  try {
    process.kill(-pid, 'SIGKILL')
  } catch {}
}

/**
 * SIGKILLs the group and anything else started from the profile, then returns what is still
 * running a second later.
 */
async function killAll(leader, profile) {
  killGroup(leader)
  for (const pid of livePids(leader, profile)) {
    try {
      process.kill(Number(pid), 'SIGKILL')
    } catch {}
  }
  const deadline = Date.now() + 1_000
  let left = livePids(leader, profile)
  while (left.length > 0 && Date.now() < deadline) {
    await sleep(20)
    left = livePids(leader, profile)
  }
  return left
}

/**
 * Every process in the browser's group or started from its profile, the browser excepted, as
 * pid → type, from one `ps` listing.
 */
function groupProcesses(leader, profile) {
  const listing = execFileSync('ps', ['-Ao', 'pid=,pgid=,command='], {
    encoding: 'utf8',
    maxBuffer: 64 * 1024 * 1024,
  })
  const processes = new Map()
  for (const line of listing.split('\n')) {
    const match = /^\s*(\d+)\s+(\d+)\s+(.*)$/.exec(line)
    if (!match) continue
    const [, pid, pgid, command] = match
    if (pid === String(leader)) continue
    if (pgid === String(leader) || command.includes(profile))
      processes.set(pid, commandType(command))
  }
  return processes
}

/** The pids still running in the browser's group or from its profile, the browser excepted. */
function livePids(leader, profile) {
  const pids = new Set([
    ...processIds(['-g', String(leader)]),
    ...processIds(['-f', profile]),
  ])
  pids.delete(String(leader))
  return [...pids]
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

/** A Chromium process's `--type=` (renderer, gpu-process, utility…), else its executable. */
function commandType(command) {
  return (
    /--type=(\S+)/.exec(command)?.[1] ??
    command.split(' --')[0].split('/').pop()
  )
}

/** `commandType` for a pid outside the listing `groupProcesses` took. */
function processType(pid) {
  try {
    return commandType(
      execFileSync('ps', ['-o', 'command=', '-p', pid], {
        encoding: 'utf8',
        stdio: ['ignore', 'pipe', 'ignore'],
      }).trim(),
    )
  } catch {
    return 'already gone'
  }
}

/** A minimal CDP client on the browser's WebSocket, separate from Playwright's. */
export async function connectCdp(cleanups, port) {
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

/**
 * Runs `cleanups` in reverse, so each resource outlives what was built on it. Every cleanup runs
 * even when one throws: a skipped one (the fixture server's close) would keep Node alive after
 * the test. Then it throws what failed.
 */
export async function runCleanups(cleanups) {
  const failures = []
  for (const cleanup of cleanups.reverse()) {
    try {
      await cleanup()
    } catch (error) {
      failures.push(error)
    }
  }
  if (failures.length === 1) throw failures[0]
  if (failures.length > 1)
    throw new AggregateError(failures, `${failures.length} cleanups failed`)
}

export function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms))
}
