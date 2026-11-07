// page.goto through the Playwright reference and through Neo, on one BrowserOS neo: one script
// navigates a fresh tab through every kind of answer and describes each one completely, the
// Response and its Request chain, so the test compares every value, its type, the objects'
// identities and their JSON between the engines. Timings differ between any two navigations, so
// the script reports their shape and invariants instead; every other value must be equal.
//
// The fixture answers without a Date header and with `Cache-Control: no-store`, so Neo's turn,
// which runs after Playwright's in the same profile, meets the same responses and no cache, and
// its cookies expire at once, so no turn sends the other's cookies.
import assert from 'node:assert/strict'
import { once } from 'node:events'
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import http from 'node:http'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { test } from 'node:test'
import {
  browserBinary,
  buildNeoRun,
  chromium,
  connectCdp,
  launchBrowser,
  playwrightVersion,
  runCleanups,
  skip,
  sleep,
  startNeo,
} from './harness.mjs'

const routes = {
  '/ok': (response) => {
    response.writeHead(200, 'Fine', [
      ['Content-Type', 'text/html; charset=utf-8'],
      ['X-Mixed-Case', 'A'],
      ['x-dup', '1'],
      ['X-Dup', '2'],
      ['Set-Cookie', 'a=1; Max-Age=0'],
      ['Set-Cookie', 'b=2; Max-Age=0'],
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
    response.end(Buffer.from('<b>\xe9</b>', 'latin1'))
  },
  '/slow-body': (response) => {
    response.writeHead(200, {
      'content-type': 'text/html',
      'content-length': 42,
    })
    response.write('<title>slow</title>')
    setTimeout(() => response.end('<p>the rest arrives</p>'), 300)
  },
  '/204': (response) => {
    response.writeHead(204)
    response.end()
  },
  '/download': (response) => {
    response.writeHead(200, { 'content-type': 'application/octet-stream' })
    response.end(Buffer.from([0xff, 0xfe, 0x41]))
  },
}

/** Adds `Cache-Control: no-store` to whichever header form a route passes `writeHead`. Node
 * refuses `setHeader` before a `writeHead` given an array of pairs. */
function noStore(response) {
  const writeHead = response.writeHead.bind(response)
  response.writeHead = (status, ...rest) => {
    const headers = rest.at(-1)
    if (Array.isArray(headers)) headers.push(['Cache-Control', 'no-store'])
    else if (headers && typeof headers === 'object')
      headers['Cache-Control'] = 'no-store'
    else rest.push({ 'Cache-Control': 'no-store' })
    return writeHead(status, ...rest)
  }
}

const source = `
  const timing = (t) => ({
    keys: Object.keys(t),
    numbers: Object.values(t).every((value) => typeof value === 'number'),
    started: t.startTime > 0,
    ended: t.responseEnd >= t.responseStart && t.responseStart >= 0,
  })
  const caught = (error) => ({ error: error.name })
  const describeRequest = async (request) => ({
    url: request.url(),
    method: request.method(),
    resourceType: request.resourceType(),
    isNavigationRequest: request.isNavigationRequest(),
    headers: request.headers(),
    allHeaders: await request.allHeaders(),
    headersArray: await request.headersArray(),
    host: await request.headerValue('HOST'),
    postData: request.postData(),
    postDataBuffer: request.postDataBuffer(),
    postDataJSON: request.postDataJSON(),
    failure: request.failure(),
    serviceWorker: request.serviceWorker(),
    frameIsMain: request.frame() === page.mainFrame(),
    timing: timing(request.timing()),
    sameTiming: request.timing() === request.timing(),
    sizes: await request.sizes().catch(caught),
    redirectedTo: request.redirectedTo()?.url() ?? null,
    sameResponse: request.existingResponse() === (await request.response()),
    json: JSON.parse(JSON.stringify(request))._type,
  })
  const describeResponse = async (response) => ({
    url: response.url(),
    ok: response.ok(),
    status: response.status(),
    statusText: response.statusText(),
    headers: response.headers(),
    allHeaders: await response.allHeaders(),
    headersArray: await response.headersArray(),
    dup: await response.headerValue('X-DUP'),
    cookies: await response.headerValues('set-cookie'),
    missing: await response.headerValue('nope'),
    fromServiceWorker: response.fromServiceWorker(),
    serverAddr: await response.serverAddr(),
    securityDetails: await response.securityDetails(),
    httpVersion: await response.httpVersion(),
    finished: await response.finished(),
    frameIsMain: response.frame() === page.mainFrame(),
    body: await response.body().then(
      (body) => ({ base64: body.toString('base64'), length: body.length, text: body.toString() }),
      caught,
    ),
    text: await response.text().catch(caught),
    json: await response.json().catch(caught),
    type: JSON.parse(JSON.stringify(response))._type,
  })
  const describe = async (response) => {
    if (response === null) return null
    const chain = []
    for (let request = response.request(); request; request = request.redirectedFrom())
      chain.unshift(request)
    const hops = []
    for (const request of chain) {
      const hop = await request.response()
      hops.push({ request: await describeRequest(request), response: hop && (await describeResponse(hop)) })
    }
    return { hops, final: (await chain.at(-1).response()) === response, title: await page.title() }
  }
  const results = {}
  for (const [name, url, options] of CASES) {
    results[name] = await page.goto(url, options).then(describe, caught)
  }
  const left = await page.goto(ORIGIN + '/json')
  await page.goto(ORIGIN + '/404')
  results.bodyAfterNavigatingAway = await left.body().then(() => 'resolved', (error) => error.name)
  return results
`

test('goto answers the same Response, Request chain and nulls as Playwright', {
  skip,
  timeout: 240_000,
}, async (t) => {
  t.diagnostic(`Playwright reference: ${playwrightVersion}`)
  const binary = buildNeoRun()
  const cleanups = []
  try {
    const server = http.createServer((request, response) => {
      response.sendDate = false
      noStore(response)
      const route = routes[request.url.split('?')[0]]
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
    const cases = [
      ['ok', `${origin}/ok`],
      ['sameDocument', `${origin}/ok#elsewhere`],
      ['redirects', `${origin}/r1`],
      ['notFound', `${origin}/404`],
      ['json', `${origin}/json`],
      ['latin1', `${origin}/latin`],
      ['commitBeforeBody', `${origin}/slow-body`, { waitUntil: 'commit' }],
      [
        'domContentLoaded',
        `${origin}/ok?dcl`,
        { waitUntil: 'domcontentloaded' },
      ],
      ['noContent', `${origin}/204`],
      ['download', `${origin}/download`],
      ['blank', 'about:blank'],
      ['data', 'data:text/html,<title>d</title>'],
    ]
    const script = `const ORIGIN = ${JSON.stringify(origin)};\nconst CASES = ${JSON.stringify(cases)};\n${source}`
    const { port } = await launchBrowser(cleanups, browserBinary)
    const cdp = await connectCdp(cleanups, port)
    const browser = await chromium.connectOverCDP(`http://127.0.0.1:${port}`)
    cleanups.push(() => browser.close())
    const page = await browser.contexts()[0].newPage()
    const reference = await new (async () => {}).constructor('page', script)(
      page,
    )
    await page.close()
    // Expired cookies and finished loads settle before Neo's turn.
    await sleep(500)

    const { targetId } = await cdp.send('Target.createTarget', {
      url: 'about:blank',
    })
    cleanups.push(() =>
      cdp.send('Target.closeTarget', { targetId }).catch(() => {}),
    )
    const scripts = mkdtempSync(join(tmpdir(), 'neo-goto-parity-'))
    cleanups.push(() => rmSync(scripts, { recursive: true, force: true }))
    const path = join(scripts, 'goto.js')
    writeFileSync(path, script)
    const outcome = await startNeo(binary, port, targetId, path).outcome
    assert.equal(outcome.status, 'ok', JSON.stringify(outcome))

    // Playwright's own answers, so equal values cannot both be wrong the same way.
    const final = (name) => reference[name].hops.at(-1)
    assert.deepEqual(
      reference.redirects.hops.map(({ request, response }) => [
        request.url,
        response.status,
      ]),
      [
        [`${origin}/r1`, 302],
        [`${origin}/r2`, 301],
        [`${origin}/ok`, 200],
      ],
    )
    assert.deepEqual(
      [
        final('ok').response.statusText,
        final('ok').response.dup,
        final('ok').response.cookies,
      ],
      ['Fine', '2, 1', ['a=1; Max-Age=0', 'b=2; Max-Age=0']],
    )
    assert.deepEqual(
      [final('notFound').response.ok, final('notFound').response.status],
      [false, 404],
    )
    assert.deepEqual(final('json').response.json, { a: [1, 2] })
    assert.equal(
      final('latin1').response.text,
      '<b>é</b>',
      'Chromium decodes, the body is UTF-8',
    )
    assert.equal(
      final('commitBeforeBody').response.text,
      '<title>slow</title><p>the rest arrives</p>',
    )
    assert.deepEqual(
      [reference.sameDocument, reference.blank, reference.data],
      [null, null, null],
    )
    assert.deepEqual(
      [reference.noContent, reference.download],
      [{ error: 'Error' }, { error: 'Error' }],
    )

    for (const name of Object.keys(reference))
      assert.deepEqual(
        outcome.value[name],
        reference[name],
        `${name}: Neo vs Playwright`,
      )
    assert.deepEqual(outcome.value, reference)
  } finally {
    await runCleanups(cleanups)
  }
})
