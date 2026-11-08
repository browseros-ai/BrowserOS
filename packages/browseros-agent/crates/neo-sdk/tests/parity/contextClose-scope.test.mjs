// context.close() in a run that goes on: what the run set on the user's tab is gone before close()
// resolves (Neo's `Tabs::release_run_scoped`). Closing the browser's default context through
// connectOverCDP drops Playwright's CDP connection (contextClose.test.mjs), and with it every
// override its sessions carried: the tab it leaves open sends no extra header, is online, and has
// the geolocation permission back to the profile's own. Neo's realm can outlive the close, so the
// test reads the tab over its own CDP client while the run still holds its sessions.
//
// Each turn, on a tab the test opened as a user's tab: the script sets an extra header, a
// geolocation grant and a position, then waits on the page (`hold('a')`) while the test reads that
// each reached the tab; it goes offline and waits again (`hold('b')`) while the test reads that;
// then it closes the context. Playwright's script returns there; Neo's keeps its realm for
// KEEP_ALIVE_MS more, longer than the test reads the tab for (GONE_WITHIN_MS), so a release that
// waits for the run's end shows as the run's state still on the tab.
//
// Runs under Node, not Bun, on the shared harness (harness.mjs).

import assert from 'node:assert/strict'
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { test } from 'node:test'
import {
  browserBinary,
  buildNeoRun,
  chromium,
  connectCdp,
  eventually,
  launchBrowser,
  openTab,
  playwrightVersion,
  runCleanups,
  runInPlaywright,
  serveFixture,
  skip,
  sleep,
  startNeo,
} from './harness.mjs'

/** How long Neo's realm lives on after close() resolves. */
const KEEP_ALIVE_MS = 15_000
/** How long the test reads the tab for the run's state to go, from the last hold's release. */
const GONE_WITHIN_MS = 10_000

const script = (
  keepAliveMs,
) => `const origin = page.url().match(/^[a-z]+:\\/\\/[^/]+/)[0]
const hold = (name) => page.evaluate((name) => new Promise((resolve) => { window['release_' + name] = resolve }), name)
await context.setExtraHTTPHeaders({ 'x-scope': 'owned' })
await context.grantPermissions(['geolocation'], { origin })
await context.setGeolocation({ latitude: 10, longitude: 20 })
await hold('a')
await context.setOffline(true)
await hold('b')
await context.close()
${keepAliveMs ? `await new Promise((resolve) => setTimeout(resolve, ${keepAliveMs}))` : ''}
return 'closed'`

/**
 * What the tab shows of the run, read over the test's own session: the extra header a fetch of
 * /probe carries (`none` without one, the error's name when the fetch fails), navigator.onLine,
 * the geolocation permission, and with `position` the position the page reads.
 */
const READ = (position) => `(async () => ({
  header: await fetch('/probe', { cache: 'no-store' }).then((response) => response.text(), (error) => error.name),
  onLine: navigator.onLine,
  permission: await navigator.permissions.query({ name: 'geolocation' }).then((status) => status.state),
  ${
    position
      ? `position: await new Promise((resolve) => navigator.geolocation.getCurrentPosition(
    (where) => resolve([where.coords.latitude, where.coords.longitude]),
    (error) => resolve('error ' + error.code),
    { timeout: 2000 },
  )),`
      : ''
  }
}))()`

const HELD = {
  a: {
    header: 'owned',
    onLine: true,
    permission: 'granted',
    position: [10, 20],
  },
  b: { header: 'TypeError', onLine: false, permission: 'granted' },
}

test("context.close() takes the run's session overrides off the tab it leaves open, as Playwright's does", {
  skip,
  timeout: 180_000,
}, async (t) => {
  t.diagnostic(`Playwright ${playwrightVersion}`)
  const neoRun = buildNeoRun()
  const cleanups = []
  try {
    const origin = await serveFixture(cleanups, {
      pages: {
        '/scope.html': '<!doctype html><title>scope</title><p>scope</p>',
        '/probe': (request, response) => {
          response.setHeader('content-type', 'text/plain')
          response.setHeader('cache-control', 'no-store')
          response.end(request.headers['x-scope'] ?? 'none')
        },
      },
    })
    const browser = await launchBrowser(cleanups, browserBinary)
    const cdp = await connectCdp(cleanups, browser.port)
    const scripts = mkdtempSync(join(tmpdir(), 'neo-context-close-scope-'))
    cleanups.push(() => rmSync(scripts, { recursive: true, force: true }))

    const playwright = await turn(
      cdp,
      `${origin}/scope.html`,
      async (targetId) => {
        const connection = await chromium.connectOverCDP(
          `http://127.0.0.1:${browser.port}`,
        )
        try {
          const [context] = connection.contexts()
          return await runInPlaywright(context, targetId, script(0))
        } finally {
          await connection.close()
        }
      },
    )
    assert.deepEqual(
      playwright.held,
      HELD,
      "the run's state reached Playwright's tab",
    )
    assert.deepEqual(playwright.result, { value: 'closed' }, 'Playwright')

    const scriptPath = join(scripts, 'scope.js')
    writeFileSync(scriptPath, script(KEEP_ALIVE_MS))
    const neo = await turn(cdp, `${origin}/scope.html`, async (targetId) => {
      const run = startNeo(neoRun, browser.port, targetId, scriptPath)
      await run.connected
      const outcome = await run.outcome
      return outcome.status === 'ok'
        ? { value: outcome.value }
        : { error: outcome.error }
    })
    assert.deepEqual(neo.held, HELD, "the run's state reached Neo's tab")
    assert.deepEqual(neo.result, { value: 'closed' }, 'Neo')
    assert.deepEqual(
      neo.afterClose,
      playwright.afterClose,
      'what the tab shows once close() resolved',
    )
  } finally {
    await runCleanups(cleanups)
  }
})

/**
 * Opens the turn's tab, starts `run` on it, and reads the tab at each hold before releasing it;
 * after the last, reads the tab until the run's state is gone from it or GONE_WITHIN_MS passed,
 * while `run` goes on; then waits for `run` and closes the tab.
 */
async function turn(cdp, url, run) {
  const { targetId, sessionId } = await openTab(cdp, url)
  try {
    await eventually(
      async () =>
        (await settle(cdp, sessionId, 'document.readyState')) === 'complete',
      `${url} loaded`,
    )
    const running = run(targetId)
    // Rejects as the run does, so a run that fails before a hold ends the wait for it; awaited
    // only there, so it is marked handled for after the holds.
    const failed = running.then(() => new Promise(() => {}))
    failed.catch(() => {})
    const held = {}
    for (const name of ['a', 'b']) {
      await Promise.race([
        eventually(
          async () =>
            (await settle(cdp, sessionId, `typeof window.release_${name}`)) ===
            'function',
          `the script holds at ${name}`,
        ),
        failed,
      ])
      held[name] = await settle(cdp, sessionId, READ(name === 'a'))
      await settle(cdp, sessionId, `window.release_${name}()`)
    }
    const until = Date.now() + GONE_WITHIN_MS
    let afterClose = await settle(cdp, sessionId, READ(false))
    while (stillTheRuns(afterClose) && Date.now() < until) {
      await sleep(50)
      afterClose = await settle(cdp, sessionId, READ(false))
    }
    const result = await running
    return { held, afterClose, result }
  } finally {
    await cdp.send('Target.closeTarget', { targetId }).catch(() => {})
  }
}

/** Whether the tab still shows any of what the run set. */
function stillTheRuns({ header, onLine, permission }) {
  return (
    header === 'owned' ||
    header === 'TypeError' ||
    !onLine ||
    permission === 'granted'
  )
}

async function settle(cdp, sessionId, expression) {
  const reply = await cdp.send(
    'Runtime.evaluate',
    { expression, returnByValue: true, awaitPromise: true },
    sessionId,
  )
  assert.equal(
    reply.exceptionDetails,
    undefined,
    JSON.stringify(reply.exceptionDetails),
  )
  return reply.result.value
}
