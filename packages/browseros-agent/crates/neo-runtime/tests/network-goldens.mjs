// Records what Playwright's `page.goto` returns next to the CDP events it was built from, for
// `src/network.rs`'s tests: real Playwright (the pinned build, or npm playwright-core) drives each
// navigation on a headless BrowserOS neo, while a second CDP session on the same tab, with Page and
// Network enabled as Neo enables them, records the events Neo's page watcher reads: every Network
// event, and the main frame's commits and lifecycle. Both sessions see the same events, so
// `network.rs` must rebuild Playwright's values from these events alone.
//
//   NEO_E2E_BROWSER=… NEO_PARITY_PLAYWRIGHT=… node crates/neo-runtime/tests/network-goldens.mjs
//
// Writes tests/fixtures/network-goldens.json. Each case keeps the session's events from the goto's
// start until its response finished (or a settle delay), and Playwright's answer: `null`, the
// error's name, or every Response and Request value Neo serves.

import { once } from 'node:events'
import { writeFileSync } from 'node:fs'
import http from 'node:http'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const here = dirname(fileURLToPath(import.meta.url))
const harness = join(here, '../../neo-sdk/tests/parity/harness.mjs')
const { chromium, launchBrowser, playwrightVersion, runCleanups, sleep } =
  await import(harness)

/** Every route answers without a Date header, so a rerun only moves ports and timings. */
const routes = {
  '/ok': (response) => {
    response.writeHead(200, 'Fine', [
      ['Content-Type', 'text/html; charset=utf-8'],
      ['X-Mixed-Case', 'A'],
      ['x-dup', '1'],
      ['X-Dup', '2'],
      ['Set-Cookie', 'a=1'],
      ['Set-Cookie', 'b=2'],
    ])
    response.end('<title>ok</title>héllo')
  },
  '/r1': (response) => {
    response.writeHead(302, { Location: '/r2' })
    response.end('moved')
  },
  '/r2': (response) => {
    response.writeHead(301, { Location: '/ok#frag' })
    response.end()
  },
  '/404': (response) => {
    response.writeHead(404, { 'content-type': 'text/html' })
    response.end('<title>nf</title>missing')
  },
  '/json': (response) => {
    response.writeHead(200, { 'content-type': 'application/json' })
    response.end('{"a":[1,2]}')
  },
  '/latin': (response) => {
    response.writeHead(200, { 'content-type': 'text/html; charset=iso-8859-1' })
    response.end(Buffer.from([0x3c, 0x62, 0x3e, 0xe9, 0x3c, 0x2f, 0x62, 0x3e]))
  },
  '/slow-body': (response) => {
    response.writeHead(200, {
      'content-type': 'text/html',
      'content-length': 40,
    })
    response.write('<title>slow</title>')
    setTimeout(() => response.end('<p>the rest arrives</p>'), 400)
  },
  '/204': (response) => {
    response.writeHead(204)
    response.end()
  },
}

const cases = [
  ['ok', '/ok'],
  ['redirects', '/r1'],
  ['not-found', '/404'],
  ['json', '/json'],
  ['latin1', '/latin'],
  ['commit-before-body', '/slow-body', { waitUntil: 'commit' }],
  ['no-content', '/204'],
  ['blank', 'about:blank'],
  ['data', 'data:text/html,<title>d</title>'],
]

const cleanups = []
try {
  const server = http.createServer((request, response) => {
    response.sendDate = false
    const route = routes[request.url]
    if (route) return route(response)
    response.writeHead(404)
    response.end()
  })
  server.listen(0, '127.0.0.1')
  await once(server, 'listening')
  cleanups.push(() => {
    server.closeAllConnections()
    server.close()
  })
  const origin = `http://127.0.0.1:${server.address().port}`
  const { port } = await launchBrowser(cleanups, process.env.NEO_E2E_BROWSER)
  const browser = await chromium.connectOverCDP(`http://127.0.0.1:${port}`)
  cleanups.push(() => browser.close())
  const context = browser.contexts()[0]
  const page = await context.newPage()
  const cdp = await context.newCDPSession(page)
  const { targetInfo } = await cdp.send('Target.getTargetInfo')
  const recorder = await recordEvents(port, targetInfo.targetId, cleanups)

  const results = []
  for (const [name, path, options] of cases) {
    const url = path.startsWith('/') ? origin + path : path
    recorder.events.length = 0
    let answer
    try {
      const response = await page.goto(url, options)
      answer = response ? await describe(response, page) : null
    } catch (error) {
      answer = { error: error.name }
    }
    // Let a body still loading at commit finish, so its loadingFinished is recorded too.
    await sleep(600)
    if (answer && !answer.error)
      answer.timingAfterSettle = answer.request.timingAfter()
    if (answer?.request) delete answer.request.timingAfter
    results.push({
      name,
      url,
      options: options ?? {},
      mainFrame: targetInfo.targetId,
      events: recorder.events.splice(0),
      playwright: answer,
    })
  }
  const file = join(here, 'fixtures/network-goldens.json')
  writeFileSync(
    file,
    `${JSON.stringify({ playwright: playwrightVersion, cases: results }, null, 1)}\n`,
  )
  console.log(
    `wrote ${file}: ${results.length} cases, Playwright ${playwrightVersion}`,
  )
} finally {
  await runCleanups(cleanups)
}

/** A second CDP client on the tab, enabled as Neo enables a session before its first goto. */
async function recordEvents(port, targetId, cleanups) {
  const version = await (
    await fetch(`http://127.0.0.1:${port}/json/version`)
  ).json()
  const socket = new WebSocket(version.webSocketDebuggerUrl)
  await once(socket, 'open')
  cleanups.push(() => socket.close())
  let nextId = 0
  const pending = new Map()
  const recorder = { events: [] }
  let sessionId
  socket.addEventListener('message', ({ data }) => {
    const message = JSON.parse(data)
    if (message.id !== undefined) {
      pending.get(message.id)?.(message)
      pending.delete(message.id)
    } else if (message.sessionId === sessionId && kept(message)) {
      recorder.events.push({ method: message.method, params: message.params })
    }
  })
  const kept = ({ method, params }) =>
    method.startsWith('Network.') ||
    method === 'Page.frameNavigated' ||
    method === 'Page.navigatedWithinDocument' ||
    (method === 'Page.lifecycleEvent' && params.frameId === targetId)
  const send = (method, params = {}, session) =>
    new Promise((resolve) => {
      const id = ++nextId
      pending.set(id, resolve)
      socket.send(JSON.stringify({ id, method, params, sessionId: session }))
    })
  ;({
    result: { sessionId },
  } = await send('Target.attachToTarget', { targetId, flatten: true }))
  await send('Page.enable', {}, sessionId)
  await send('Page.setLifecycleEventsEnabled', { enabled: true }, sessionId)
  await send('Network.enable', {}, sessionId)
  return recorder
}

async function describe(response, page) {
  const chain = []
  for (
    let request = response.request();
    request;
    request = request.redirectedFrom()
  )
    chain.unshift(request)
  const hops = []
  for (const request of chain) {
    const hop = await request.response()
    hops.push({
      request: {
        url: request.url(),
        method: request.method(),
        resourceType: request.resourceType(),
        isNavigationRequest: request.isNavigationRequest(),
        headers: request.headers(),
        headersArray: await request.headersArray(),
        postData: request.postData(),
        failure: request.failure(),
        timing: { ...request.timing() },
        sizes: await request.sizes().catch((error) => ({ error: error.name })),
      },
      response: hop && {
        url: hop.url(),
        ok: hop.ok(),
        status: hop.status(),
        statusText: hop.statusText(),
        headers: hop.headers(),
        headersArray: await hop.headersArray(),
        fromServiceWorker: hop.fromServiceWorker(),
        serverAddr: await hop.serverAddr(),
        securityDetails: await hop.securityDetails(),
        httpVersion: await hop.httpVersion(),
        body: await hop.body().then(
          (body) => body.toString('base64'),
          (error) => ({ error: error.name }),
        ),
      },
    })
  }
  const last = chain.at(-1)
  return {
    hops,
    frameIsMain: response.frame() === page.mainFrame(),
    request: { timingAfter: () => ({ ...last.timing() }) },
  }
}
