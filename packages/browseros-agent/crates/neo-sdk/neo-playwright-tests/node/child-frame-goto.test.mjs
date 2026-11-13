// biome-ignore-all lint/suspicious/noUndeclaredEnvVars: Standalone native browser regression.
/** Keep a logical frame and its response correlated while Chromium changes its renderer. */
import assert from 'node:assert/strict'
import fs from 'node:fs'
import http from 'node:http'
import os from 'node:os'
import path from 'node:path'
import test from 'node:test'
import { launchBrowser } from './browser.mjs'
import { openClaw } from './claw-loader.mjs'

function respond(request, response) {
  response.setHeader('content-type', 'text/html')
  response.setHeader('cache-control', 'no-store')
  if (request.url === '/parent')
    response.end('<iframe name="child" src="/start"></iframe>')
  else if (request.url === '/remote-parent')
    response.end('<iframe name="grandchild" src="/start"></iframe>')
  else if (request.url === '/redirect')
    response.end('<script>location.href="/replacement"</script>')
  else {
    response.statusCode = request.url === '/created' ? 201 : 200
    response.end('<p>document</p>')
  }
}

async function startServer() {
  const server = http.createServer(respond)
  await new Promise((resolve, reject) => {
    server.once('error', reject)
    server.listen(0, '127.0.0.1', resolve)
  })
  return server
}

/** Every resource is private to this fixture; cleanup also runs after partial startup. */
async function closeFixture(sdk, browser, server) {
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

async function withFixture(run) {
  const output = fs.mkdtempSync(
    path.join(
      process.env.NEO_NODE_OUTPUT_DIR || os.tmpdir(),
      'child-frame-goto-',
    ),
  )
  const server = await startServer()
  let browser
  let sdk
  try {
    browser = await launchBrowser(process.env.NEO_E2E_BROWSER, output)
    sdk = await openClaw(browser.port, {
      stateDir: path.join(output, 'state'),
      timeoutMs: 60_000,
    })
    const { port } = server.address()
    await run(sdk, `http://127.0.0.1:${port}`, `http://localhost:${port}`)
  } finally {
    await closeFixture(sdk, browser, server)
  }
}

async function checkNavigation(frame, url, page, parentUrl) {
  const response = await frame.goto(url, { timeout: 15_000 })
  assert.ok(response, `HTTP navigation to ${url} has a response`)
  assert.equal(response.status(), 201)
  assert.equal(response.url(), url)
  assert.equal(response.frame(), frame)
  assert.equal(frame.url(), url)
  assert.equal(page.url(), parentUrl)
}

async function simpleNavigations(page, local, remote) {
  const child = page.frames().find((frame) => frame.name() === 'child')
  assert.ok(child)
  await checkNavigation(child, `${local}/created`, page, `${local}/parent`)
  assert.equal(
    await child.goto(`${local}/created#fragment`, { timeout: 15_000 }),
    null,
  )
  assert.equal(child.url(), `${local}/created#fragment`)
  // Different sites force both directions of renderer handoff while retaining one Frame.
  await checkNavigation(child, `${remote}/created`, page, `${local}/parent`)
  await checkNavigation(child, `${local}/created`, page, `${local}/parent`)
  return child
}

async function nestedNavigations(page, child, local, remote) {
  await child.goto(`${remote}/remote-parent`, { timeout: 15_000 })
  const grandchild = page
    .frames()
    .find((frame) => frame.name() === 'grandchild')
  assert.ok(grandchild)
  // A same-process child of an OOPIF uses that renderer, then becomes an OOPIF itself.
  await checkNavigation(
    grandchild,
    `${remote}/created`,
    page,
    `${local}/parent`,
  )
  await checkNavigation(grandchild, `${local}/created`, page, `${local}/parent`)
  assert.equal(child.url(), `${remote}/remote-parent`)
  assert.equal(await child.goto('about:blank', { timeout: 15_000 }), null)
  assert.equal(page.url(), `${local}/parent`)
}

async function specialUrls(page, child, local) {
  const error = await child
    .goto('javascript:window.__childNavigation = 1', { timeout: 5000 })
    .catch((error) => error)
  assert.match(error.message, /ERR_ABORTED/)
  assert.equal(await child.evaluate(() => window.__childNavigation), 1)
  assert.equal(
    await page.evaluate(() => typeof window.__childNavigation),
    'undefined',
  )
  assert.equal(
    await child.goto('data:text/html,data-child', { timeout: 5000 }),
    null,
  )
  assert.equal(
    await child.evaluate(() => document.body.textContent),
    'data-child',
  )
  assert.equal(page.url(), `${local}/parent`)
}

async function immediateRedirect(child, remote) {
  const result = await child
    .goto(`${remote}/redirect`, { waitUntil: 'commit', timeout: 5000 })
    .catch((error) => error)
  if (result && typeof result.message === 'string')
    assert.match(result.message, /interrupted by another navigation/)
  else if (result) {
    assert.equal(result.url(), `${remote}/redirect`)
    assert.equal(result.frame(), child)
  }
  // A late timing read may return null; a replacement must settle, never strand the loader.
  await child.waitForURL(`${remote}/replacement`, { timeout: 5000 })
}

test(
  'child goto preserves parent, frame identity, and response through renderer swaps',
  {
    skip: !process.env.NEO_E2E_BROWSER,
    timeout: 120_000,
  },
  async () =>
    withFixture(async (sdk, local, remote) => {
      const page = await sdk.context.newPage()
      await page.goto(`${local}/parent`)
      const child = await simpleNavigations(page, local, remote)
      await nestedNavigations(page, child, local, remote)
      await specialUrls(page, child, local)
      await immediateRedirect(child, remote)
    }),
)
