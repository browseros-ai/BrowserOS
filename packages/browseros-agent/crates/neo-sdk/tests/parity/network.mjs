// The runner the network parity files share (requestEvents, waitForRequest, waitForResponse,
// requests, networkIdle and navigationSignal.test.mjs), on the shared harness: one BrowserOS neo,
// one fixture server (`serveNetwork`) and one neo_run build per file. Each case runs its script
// through the pinned Playwright on a fresh about:blank tab (connectOverCDP, newPage), disconnects
// Playwright, then runs the same script through Neo on a fresh about:blank tab of its own. The
// script navigates itself, with `ORIGIN` (127.0.0.1) and `CROSS` (localhost, another site, so an
// out-of-process frame) in scope, and the helpers `errorCategory` (harness.mjs), `settle` and
// `path`, so both engines report the same way: `settle(promise)` is `'undefined'`, `{ value }` or
// `{ error: kind }`, never the error's wording; `path(url)` drops the origin, whose port changes
// between runs. Event orders across concurrent requests are sorted by the scripts: only the order
// Playwright promises (one request's own events) is compared as observed.
//
// A case either expects one value from both engines (`expected`), or records a deliberate
// difference: `playwright` and `neo` are each engine's own expected value.
//
// NEO_PARITY_REFERENCE_ONLY=1 runs only Playwright's turn.

import assert from 'node:assert/strict'
import { once } from 'node:events'
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import http from 'node:http'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import {
  browserBinary,
  buildNeoRun,
  connectCdp,
  connectReference,
  errorCategory,
  launchBrowser,
  playwrightVersion,
  referenceHalf,
  runCleanups,
  runInNeo,
  sleep,
} from './harness.mjs'

const referenceOnly = Boolean(process.env.NEO_PARITY_REFERENCE_ONLY)

const PRELUDE = `const errorCategory = ${errorCategory.toString()};
const settle = (promise) => promise.then(
  (value) => (value === undefined ? 'undefined' : { value }),
  (error) => ({ error: errorCategory(error) }),
);
const path = (url) => url.replace(/^[a-z]+:\\/\\/[^/]+/, '');
`

/** A one-pixel PNG. */
const PNG = Buffer.from(
  '89504e470d0a1a0a0000000d4948445200000001000000010806000000' +
    '1f15c4890000000d49444154789c6300010000000500010d0a2db40000000049454e44ae426082',
  'hex',
)

const PAGES = {
  '/net/blank.html': '<title>blank</title>',
  // A subresource image, a fetch and a same-origin frame with a fetch of its own.
  '/net/page.html': `<title>page</title><img src="/net/img.png">
<iframe name="child" src="/net/frame.html"></iframe>
<script>fetch('/net/data.json').then((r) => r.text())</script>`,
  '/net/frame.html': `<title>frame</title><a id="next" href="/net/frame2.html">next</a>
<a id="broken" href="/net/hangup">broken</a>
<script>fetch('/net/in-frame.json').then((r) => r.text())</script>`,
  '/net/frame2.html': '<title>frame2</title>',
  '/net/frame-top.html':
    '<title>frame top</title><iframe name="child" src="/net/frame.html"></iframe>',
  // A fetch 300 ms after load whose response takes 700 ms; the title says when it finished.
  '/net/late.html': `<title>late</title><script>
setTimeout(() => fetch('/net/slow?ms=700').then((r) => r.text()).then(() => { document.title = 'done' }), 300)
</script>`,
  // The late fetch is the child frame's: the parent's networkidle waits for its frames.
  '/net/late-frame.html':
    '<title>late frame</title><iframe src="/net/late-child.html"></iframe>',
  '/net/late-child.html': `<title>late child</title><script>
setTimeout(() => fetch('/net/slow?ms=700').then((r) => r.text()).then(() => { parent.document.title = 'done' }), 300)
</script>`,
  // An image whose response never completes holds networkidle until the page replaces it.
  '/net/busy.html': '<title>busy</title><img src="/net/hold?id=blocker">',
  // A frame from localhost, another site, so Chromium runs it in its own process.
  '/net/oopif-top.html': `<title>oopif top</title><body><script>
const frame = document.createElement('iframe')
frame.name = 'cross'
frame.src = 'http://localhost:' + location.port + '/net/oopif-child.html'
document.body.append(frame)
</script>`,
  '/net/oopif-child.html': `<title>oopif child</title>
<button id="fetch" onclick="fetch('/net/oopif-data.json').then((r) => r.text())">fetch</button>`,
  // Counts its loads in sessionStorage, so a script can tell whether a call loaded it.
  '/net/counted.html': `<title>counted</title><script>
sessionStorage.setItem('loads', String(Number(sessionStorage.getItem('loads')) + 1))
</script>`,
  // An EventSource never holds networkidle.
  '/net/event-source.html': `<title>events</title><script>new EventSource('/net/sse')</script>`,
}

/**
 * The fixture server: `PAGES`, JSON at `*.json`, a PNG at `/net/img.png`, a 302 at
 * `/net/redirect` (to `?to=`, else `/net/data.json`), `/net/headers` echoing the request's method,
 * headers and body as JSON, a 204 at `/net/204`, a 404 at `/net/404`, a connection
 * dropped before any response at `/net/hangup`, `/net/slow?ms=N` after N ms (readable from
 * any origin), an event stream
 * that never ends at `/net/sse`, `/net/echo` answering a POST's body, and `/net/hold?id=X`, which
 * sends `first` (with `&type=html`, the start of an HTML document a frame can stream) and holds the
 * response open until `/net/release?id=X` ends it with `last`. Each
 * answer forbids
 * caching, so a second engine's turn makes the same requests, except `/net/cached.json`, which
 * a script fetches under a query of its own.
 */
export async function serveNetwork(cleanups) {
  /** Responses `/net/hold?id=` keeps open after their first chunk, by id. */
  const held = new Map()
  const server = http.createServer(async (request, response) => {
    const [pathname, query = ''] = request.url.split('?')
    response.setHeader('cache-control', 'no-store')
    if (pathname === '/net/hold') {
      const params = new URLSearchParams(query)
      const html = params.get('type') === 'html'
      response.writeHead(200, {
        'content-type': html ? 'text/html' : 'text/plain',
      })
      response.write(
        html
          ? `<!doctype html><title>loading</title>${' '.repeat(2048)}`
          : 'first',
      )
      held.set(params.get('id'), response)
      return
    }
    if (pathname === '/net/release') {
      const id = new URLSearchParams(query).get('id')
      held.get(id)?.end('last')
      held.delete(id)
      response.end('released')
      return
    }
    if (pathname === '/net/hangup') {
      request.socket.destroy()
      return
    }
    if (pathname === '/net/slow') {
      response.setHeader('access-control-allow-origin', '*')
      await sleep(Number(new URLSearchParams(query).get('ms') ?? 0))
      response.end('slow')
      return
    }
    if (pathname === '/net/redirect') {
      const to = new URLSearchParams(query).get('to') ?? '/net/data.json'
      response.writeHead(302, { location: to })
      response.end()
      return
    }
    if (pathname === '/net/headers') {
      let body = ''
      for await (const chunk of request) body += chunk
      response.setHeader('content-type', 'application/json')
      response.end(
        JSON.stringify({
          method: request.method,
          headers: request.headers,
          body,
        }),
      )
      return
    }
    if (pathname === '/net/204') {
      response.writeHead(204)
      response.end()
      return
    }
    if (pathname === '/net/404') {
      response.writeHead(404, { 'content-type': 'text/plain' })
      response.end('missing')
      return
    }
    if (pathname === '/net/sse') {
      response.writeHead(200, { 'content-type': 'text/event-stream' })
      response.write('data: hello\n\n')
      return
    }
    if (pathname === '/net/echo') {
      let body = ''
      for await (const chunk of request) body += chunk
      response.setHeader('content-type', 'text/plain')
      response.end(`${request.method} ${body}`)
      return
    }
    if (pathname === '/net/img.png') {
      response.setHeader('content-type', 'image/png')
      response.end(PNG)
      return
    }
    if (pathname === '/net/cached.json') {
      response.setHeader('cache-control', 'public, max-age=600')
      response.setHeader('content-type', 'application/json')
      response.end(JSON.stringify({ path: pathname }))
      return
    }
    if (pathname.endsWith('.json')) {
      response.setHeader('content-type', 'application/json')
      response.end(JSON.stringify({ path: pathname, query }))
      return
    }
    response.setHeader('content-type', 'text/html; charset=utf-8')
    response.end(PAGES[pathname] ?? `<title>${pathname}</title>`)
  })
  server.listen(0, '127.0.0.1')
  await once(server, 'listening')
  cleanups.push(() => {
    server.closeAllConnections()
    server.close()
  })
  const { port } = server.address()
  return {
    origin: `http://127.0.0.1:${port}`,
    cross: `http://localhost:${port}`,
  }
}

/**
 * Runs each case of `cases` as a subtest of `t` (see the header). `args` go to the browser's
 * command line, such as Playwright's own `--disable-back-forward-cache` for history steps.
 */
export async function compareNetwork(t, cases, { args = [] } = {}) {
  t.diagnostic(`Playwright ${playwrightVersion}`)
  if (referenceOnly) t.diagnostic('NEO_PARITY_REFERENCE_ONLY: Playwright only')
  const neoRun = referenceOnly ? undefined : buildNeoRun()
  const cleanups = []
  try {
    const { origin, cross } = await serveNetwork(cleanups)
    const browser = await launchBrowser(cleanups, browserBinary, args)
    const cdp = await connectCdp(cleanups, browser.port)
    const scripts = mkdtempSync(join(tmpdir(), 'neo-network-parity-'))
    cleanups.push(() => rmSync(scripts, { recursive: true, force: true }))
    let next = 1
    for (const c of cases) {
      await t.test(c.name, async () => {
        const source = `${PRELUDE}const ORIGIN = ${JSON.stringify(origin)};
const CROSS = ${JSON.stringify(cross)};
${c.source}`
        const playwright = await viaPlaywright(browser.port, source)
        const theirs = 'expected' in c ? c.expected : c.playwright
        assert.deepEqual(playwright, { value: theirs }, 'Playwright')
        if (referenceOnly) return
        const neo = await viaNeo(
          cdp,
          browser.port,
          neoRun,
          scripts,
          next++,
          source,
        )
        const ours = 'expected' in c ? c.expected : c.neo
        assert.deepEqual(
          neo.result,
          { value: ours },
          `Neo: ${JSON.stringify(neo.outcome)}`,
        )
      })
    }
  } finally {
    await runCleanups(cleanups)
  }
}

/**
 * Runs the script as Playwright does, with `page` and `context` in scope, then disconnects
 * (`referenceHalf`).
 */
async function viaPlaywright(port, source) {
  return await referenceHalf(async () => {
    const playwright = await connectReference(port)
    try {
      const [context] = playwright.contexts()
      const page = await context.newPage()
      const AsyncFunction = (async () => {}).constructor
      const result = await new AsyncFunction('page', 'context', source)(
        page,
        context,
      ).then(
        (value) => ({ value }),
        (error) => ({ error: { name: error.name, message: error.message } }),
      )
      await page.close()
      return result
    } finally {
      await playwright.close()
    }
  })
}

/** Runs the script through neo_run on a fresh about:blank tab, with no other client on it. */
async function viaNeo(cdp, port, neoRun, scripts, index, source) {
  const { targetId } = await cdp.send('Target.createTarget', {
    url: 'about:blank',
  })
  try {
    const scriptPath = join(scripts, `case-${index}.js`)
    writeFileSync(scriptPath, source)
    const { outcome } = await runInNeo(neoRun, port, targetId, scriptPath)
    const result =
      outcome.status === 'ok'
        ? { value: outcome.value }
        : { error: outcome.error }
    return { outcome, result }
  } finally {
    // A script may have closed its page already.
    await cdp.send('Target.closeTarget', { targetId }).catch(() => {})
  }
}
