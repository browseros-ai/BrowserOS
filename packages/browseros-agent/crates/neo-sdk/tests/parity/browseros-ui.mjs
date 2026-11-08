// What the BrowserOS UI parity files share (count, ariaSnapshot and dispatchEvent
// `-browseros-ui.test.mjs`): BrowserOS's bundled claw-app mounts its takeover bar
// (<browseros-neo-takeover>, a shadow host with its own <html>, <body>, input and buttons) on
// every http(s) page, and Neo must read such a page as Playwright reads it on a browser without
// the bar. So the reference is Playwright on BrowserOS with its bundled extensions off (a plain
// Chromium page), and Neo runs on BrowserOS as shipped, the real bar in place: never removed,
// required present, and in a second case shown as a parked agent shows it to the human. A
// shipped claw-app whose bar root is open proves Neo's own guard; the root's mode is reported.

import assert from 'node:assert/strict'
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import {
  browserBinary,
  buildNeoRun,
  connectCdp,
  errorCategory,
  eventually,
  launchBrowser,
  launchReference,
  openTab,
  playwrightVersion,
  referenceHalf,
  runCleanups,
  runInNeo,
  serveFixture,
} from './harness.mjs'

/** BrowserOS's bundled claw-app, the extension that mounts the bar (`bundled_extensions.json`). */
const CLAW_APP_ID = 'pjimfkbpehlcllblajnpfamdfjhhlgkc'

/**
 * The fixture: the page's own body, text box and button, plus a shadow host of its own whose
 * open root holds a second text box and button. Playwright pierces that root, so it must count.
 */
export const PAGE = `<!doctype html><html lang="en"><head><title>Own page</title></head><body>
<h1>Own page</h1>
<label>Name <input id="name"></label>
<button id="go" type="button">Go</button>
<div id="own-host"></div>
<script>
const root = document.getElementById('own-host').attachShadow({ mode: 'open' })
root.innerHTML = '<input id="inner-name" aria-label="Inner name"><button id="inner-go" type="button">Inner go</button>'
window.__clicks = []
for (const id of ['name', 'go']) document.getElementById(id).addEventListener('click', () => window.__clicks.push(id))
for (const id of ['inner-name', 'inner-go']) root.getElementById(id).addEventListener('click', () => window.__clicks.push(id))
</script>
</body></html>`

/**
 * Runs `source` (an agent script, `page` in scope) on fresh tabs of `PAGE`: Playwright's on the
 * reference browser, then Neo's with the bar hidden and with the bar shown, and returns
 * `{ playwright, hidden, shown }` with each engine's value (`{ value }` or `{ error }`), Neo's
 * `{ mode }` of the bar's root, and each tab's page-side `probe` value.
 */
export async function compareWithBar(t, { source, probe = 'null' }) {
  t.diagnostic(`Playwright reference: ${playwrightVersion}`)
  const neoRun = buildNeoRun()
  const script = `const errorCategory = ${errorCategory.toString()};\n${source}`
  const cleanups = []
  try {
    const origin = await serveFixture(cleanups, { pages: { '/page': PAGE } })
    const url = `${origin}/page`
    const { playwright: reference } = await launchReference(
      cleanups,
      browserBinary,
      { args: ['--disable-browseros-extensions'] },
    )
    const playwright = await referenceHalf(async () => {
      const page = await reference.contexts()[0].newPage()
      await page.goto(url)
      assert.equal(
        await page.evaluate(
          () => document.querySelectorAll('browseros-neo-takeover').length,
        ),
        0,
        'the reference page has no takeover bar',
      )
      const AsyncFunction = (async () => {}).constructor
      const result = await new AsyncFunction('page', script)(page).then(
        (value) => ({ value }),
        (error) => ({ error: { name: errorCategory(error) } }),
      )
      result.probe = await page.evaluate(probe)
      await page.close()
      return result
    })

    const browser = await launchBrowser(cleanups, browserBinary)
    const cdp = await connectCdp(cleanups, browser.port)
    // claw-app installs from the bundle shortly after a fresh profile starts; a page loaded
    // before its service worker runs never gets the bar.
    await eventually(
      async () =>
        (await cdp.send('Target.getTargets')).targetInfos.some((target) =>
          target.url.startsWith(`chrome-extension://${CLAW_APP_ID}/`),
        ),
      'claw-app has started',
      30_000,
    )
    const scripts = mkdtempSync(join(tmpdir(), 'neo-browseros-ui-'))
    cleanups.push(() => rmSync(scripts, { recursive: true, force: true }))
    const scriptPath = join(scripts, 'script.js')
    writeFileSync(scriptPath, script)
    const neo = {}
    for (const bar of ['hidden', 'shown']) {
      const { targetId, sessionId } = await openTab(cdp, url)
      const evaluate = async (expression) =>
        (
          await cdp.send(
            'Runtime.evaluate',
            { expression, returnByValue: true },
            sessionId,
          )
        ).result.value
      await eventually(
        () => evaluate(`!!document.querySelector('browseros-neo-takeover')`),
        'the takeover bar is mounted',
        20_000,
      )
      const mode = (await barState(cdp, sessionId)).mode
      if (bar === 'shown') await showBar(cdp, sessionId, browser.port, targetId)
      assert.equal(
        (await barState(cdp, sessionId)).shown,
        bar === 'shown',
        `the bar is ${bar}`,
      )
      const run = await runInNeo(neoRun, browser.port, targetId, scriptPath)
      neo[bar] = {
        ...(run.outcome.status === 'ok'
          ? { value: run.value }
          : { error: { name: run.error?.name, message: run.error?.message } }),
        mode,
        probe: await evaluate(probe),
        barStill: (await barState(cdp, sessionId)).shown,
      }
      await cdp.send('Target.closeTarget', { targetId })
    }
    t.diagnostic(`the bar's root on this BrowserOS: ${neo.hidden.mode}`)
    return { playwright, ...neo }
  } finally {
    await runCleanups(cleanups)
  }
}

/** Every node of a `DOM.getDocument {pierce: true}` tree, shadow roots included. */
function nodes(node) {
  return [
    node,
    ...(node.children ?? []).flatMap(nodes),
    ...(node.shadowRoots ?? []).flatMap(nodes),
  ]
}

/**
 * The bar as CDP sees it, closed roots included: its root's mode, and whether its panel shows
 * (`.nt-root` without `hidden`, claw-app's `takeover-bar.ts`).
 */
async function barState(cdp, sessionId) {
  const { root } = await cdp.send(
    'DOM.getDocument',
    { depth: -1, pierce: true },
    sessionId,
  )
  const all = nodes(root)
  const host = all.find((node) => node.nodeName === 'BROWSEROS-NEO-TAKEOVER')
  const attributes = (node) => {
    const pairs = node.attributes ?? []
    const map = {}
    for (let i = 0; i < pairs.length; i += 2) map[pairs[i]] = pairs[i + 1]
    return map
  }
  const panel = all.find((node) =>
    (attributes(node).class ?? '').split(' ').includes('nt-root'),
  )
  return {
    mode: host?.shadowRoots?.[0]?.shadowRootType ?? null,
    shown: !!panel && !('hidden' in attributes(panel)),
  }
}

/**
 * Shows the bar as a parked agent does: inside claw-app's content script world only, its poll
 * is answered with an active takeover (the background would ask the claw server, which the test
 * browser does not run). The page itself is untouched. The world is found through
 * `Runtime.enable` on a socket of the tab's own, closed again before Neo runs.
 */
async function showBar(cdp, sessionId, port, targetId) {
  const socket = await tabSocket(port, targetId)
  try {
    await socket.send('Runtime.enable')
    let world
    await eventually(() => {
      world = socket.events.find(
        (event) =>
          event.method === 'Runtime.executionContextCreated' &&
          event.params.context.origin === `chrome-extension://${CLAW_APP_ID}`,
      )?.params.context
      return !!world
    }, "claw-app's content script world is reported")
    await socket.send('Runtime.evaluate', {
      contextId: world.id,
      expression: `(() => {
        const real = chrome.runtime.sendMessage.bind(chrome.runtime)
        const context = { sessionId: 'parity', agentLabel: 'parity', reason: 'Sign in to continue',
          site: location.host, requestedAt: Date.now() }
        chrome.runtime.sendMessage = async (message, ...rest) =>
          message?.type === 'takeover-poll' ? { active: true, context }
            : message?.type === 'takeover-resolve' ? {} : real(message, ...rest)
      })()`,
    })
    await socket.send('Runtime.disable')
  } finally {
    socket.close()
  }
  // The content script polls on visibilitychange as well as every 3 s.
  await cdp.send(
    'Runtime.evaluate',
    { expression: `document.dispatchEvent(new Event('visibilitychange'))` },
    sessionId,
  )
  await eventually(
    async () => (await barState(cdp, sessionId)).shown,
    'the bar shows',
  )
}

/**
 * A CDP socket on one tab (`/devtools/page/<targetId>`) that keeps every event it receives in
 * `events`, for the one setup step that needs `Runtime` events.
 */
async function tabSocket(port, targetId) {
  const socket = new WebSocket(
    `ws://127.0.0.1:${port}/devtools/page/${targetId}`,
  )
  await new Promise((resolve, reject) => {
    socket.addEventListener('open', resolve, { once: true })
    socket.addEventListener(
      'error',
      () => reject(new Error('tab socket failed')),
      { once: true },
    )
  })
  const pending = new Map()
  const events = []
  let nextId = 1
  socket.addEventListener('message', ({ data }) => {
    const message = JSON.parse(data)
    if (message.method) events.push(message)
    const call = pending.get(message.id)
    if (!call) return
    pending.delete(message.id)
    if (message.error)
      call.reject(new Error(`${call.method}: ${message.error.message}`))
    else call.resolve(message.result)
  })
  return {
    events,
    send(method, params = {}) {
      return new Promise((resolve, reject) => {
        const id = nextId++
        pending.set(id, { method, resolve, reject })
        socket.send(JSON.stringify({ id, method, params }))
      })
    },
    close: () => socket.close(),
  }
}
