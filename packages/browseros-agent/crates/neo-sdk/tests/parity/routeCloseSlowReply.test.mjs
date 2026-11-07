// An awaited context.close() lets go of every request the run's routes hold, and stops pausing
// requests, even when the browser is slow to acknowledge one continuation: the close's cleanup
// sends every continuation and the pausing's end without waiting on one another, and owns them
// past its caller's budget (neo-runtime `tabs.rs` `Tabs::release_run_scoped`, `routes.rs`
// `finish`). Playwright's closed context releases its client's paused requests.
//
// From the #3233 re-check (pw-net-rev-b1 `routes-fix/slow-close.test.mjs`), its scenario and
// assertions unchanged: a CDP proxy holds the browser's reply to the first
// `Fetch.continueRequest` after the browser took the command; the run stays live after the close
// (`route-close-host.rs`), and its post-close reads of `context.pages` mark that the close
// resolved.
//
// Runs under Node, not Bun, on the shared harness (harness.mjs).

import assert from 'node:assert/strict'
import { spawn, spawnSync } from 'node:child_process'
import { once } from 'node:events'
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import http from 'node:http'
import { createRequire } from 'node:module'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'
import { test } from 'node:test'
import { fileURLToPath } from 'node:url'
import {
  browserBinary,
  chromium,
  connectCdp,
  eventually,
  launchBrowser,
  openTab,
  runCleanups,
  serveFixture,
  skip,
} from './harness.mjs'

const require = createRequire(import.meta.url)
const { ws: WebSocket, wsServer: WebSocketServer } = require(
  join(
    process.env.NEO_PARITY_PLAYWRIGHT ??
      dirname(require.resolve('playwright-core/package.json')),
    'lib/utilsBundle.js',
  ),
)

/** Builds `route-close-host.rs` against the crate's own libraries, as binding-cancel-review does. */
function buildHost(root) {
  const built = spawnSync(
    'cargo',
    [
      'build',
      '-p',
      'neo-runtime',
      '--example',
      'neo_run',
      '--locked',
      '--message-format=json',
    ],
    { encoding: 'utf8', maxBuffer: 64 * 1024 * 1024 },
  )
  assert.equal(built.status, 0, built.stderr)
  const libs = new Map()
  for (const line of built.stdout.split('\n')) {
    if (!line.startsWith('{')) continue
    const row = JSON.parse(line)
    if (row.reason !== 'compiler-artifact') continue
    for (const path of row.filenames ?? [])
      if (path.endsWith('.rlib')) libs.set(row.target.name, path)
  }
  const args = [
    '--edition=2024',
    '-L',
    `dependency=${dirname(libs.get('neo_runtime'))}`,
  ]
  for (const name of [
    'browseros_cdp',
    'neo_runtime',
    'tokio',
    'tokio_util',
    'serde_json',
  ]) {
    assert.ok(libs.has(name), `missing ${name} library`)
    args.push('--extern', `${name}=${libs.get(name)}`)
  }
  const bin = join(root, 'route-close-host')
  args.push(
    fileURLToPath(new URL('./route-close-host.rs', import.meta.url)),
    '-o',
    bin,
  )
  const compiled = spawnSync('rustc', args, { encoding: 'utf8' })
  assert.equal(compiled.status, 0, compiled.stderr)
  return bin
}

/** A CDP proxy that holds the browser's reply to the first `Fetch.continueRequest` and counts
 * continuations, the `Fetch.disable`s after a `Fetch.enable`, and post-close page reads. */
async function proxyFor(cleanups, upstream) {
  let port
  let held = false
  let releaseReply
  let reads = 0
  let continues = 0
  let disables = 0
  let armed = false
  const sockets = new Set()
  const server = http.createServer((_request, response) =>
    response.end(
      JSON.stringify({ webSocketDebuggerUrl: `ws://127.0.0.1:${port}/review` }),
    ),
  )
  const wss = new WebSocketServer({ server })
  wss.on('connection', (down) => {
    const up = new WebSocket(upstream)
    const queue = []
    const calls = new Map()
    sockets.add(down)
    sockets.add(up)
    up.on('open', () => {
      for (const text of queue) up.send(text)
      queue.length = 0
    })
    down.on('message', (data) => {
      const text = data.toString()
      const message = JSON.parse(text)
      calls.set(message.id, message)
      if (message.method === 'Fetch.enable') armed = true
      if (message.method === 'Fetch.continueRequest') continues++
      if (armed && message.method === 'Fetch.disable') disables++
      if (held && message.method === 'Browser.getTabs') reads++
      if (up.readyState === WebSocket.OPEN) up.send(text)
      else queue.push(text)
    })
    up.on('message', (data) => {
      const text = data.toString()
      const message = JSON.parse(text)
      const call = calls.get(message.id)
      if (!held && call?.method === 'Fetch.continueRequest') {
        held = true
        releaseReply = () => {
          if (down.readyState === WebSocket.OPEN) down.send(text)
        }
        return
      }
      if (down.readyState === WebSocket.OPEN) down.send(text)
    })
    down.on('close', () => up.close())
    up.on('close', () => down.close())
    down.on('error', () => {})
    up.on('error', () => {})
  })
  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve))
  port = server.address().port
  cleanups.push(async () => {
    releaseReply?.()
    for (const socket of sockets) socket.close()
    await new Promise((resolve) => wss.close(resolve))
    await new Promise((resolve) => server.close(resolve))
  })
  return {
    port,
    get held() {
      return held
    },
    get reads() {
      return reads
    },
    get continues() {
      return continues
    },
    get disables() {
      return disables
    },
    release() {
      releaseReply?.()
      releaseReply = null
    },
  }
}

const launchFetches = `()=>{window.finished={};window.pending=Promise.all(['one','two'].map(name=>fetch('/held/'+name).then(r=>r.text()).then(text=>window.finished[name]=text)));}`

test('a slow continuation reply cannot leave another held route paused once context.close resolves', {
  skip,
  timeout: 180_000,
}, async (t) => {
  const root = mkdtempSync(join(tmpdir(), 'neo-route-close-'))
  const cleanup = [() => rmSync(root, { recursive: true, force: true })]
  const host = buildHost(root)
  let proxy
  let child
  let pw
  try {
    const origin = await serveFixture(cleanup, {
      pages: {
        '/top': '<title>top</title>',
        '/held/one': 'one',
        '/held/two': 'two',
      },
    })
    const browser = await launchBrowser(cleanup, browserBinary)
    const cdp = await connectCdp(cleanup, browser.port)
    const read = async (session, expression) =>
      (
        await cdp.send(
          'Runtime.evaluate',
          { expression, returnByValue: true, awaitPromise: true },
          session,
        )
      ).result.value
    const first = await openTab(cdp, `${origin}/top`)
    pw = await chromium.connectOverCDP(`http://127.0.0.1:${browser.port}`)
    const page = pw
      .contexts()[0]
      .pages()
      .find((p) => p.url() === `${origin}/top`)
    const context = page.context()
    let count = 0
    await page.route('**/held/*', () => {
      count++
      return new Promise(() => {})
    })
    await page.evaluate(`(${launchFetches})()`)
    await eventually(() => count === 2, 'two reference requests held')
    await context.close()
    await eventually(
      async () =>
        Object.keys(await read(first.sessionId, 'window.finished')).length ===
        2,
      'reference requests released',
    )
    const reference = await read(first.sessionId, 'window.finished')
    await pw.close()
    pw = null
    await cdp.send('Target.closeTarget', { targetId: first.targetId })
    const second = await openTab(cdp, `${origin}/top`)
    const upstream = (
      await (
        await fetch(`http://127.0.0.1:${browser.port}/json/version`)
      ).json()
    ).webSocketDebuggerUrl
    proxy = await proxyFor(cleanup, upstream)
    const script = join(root, 'slow-close.js')
    writeFileSync(
      script,
      `let count=0;await page.route('**/held/*',()=>{count++;return new Promise(()=>{})});await page.evaluate(${launchFetches});for(let i=0;count<2&&i<500;i++)await new Promise(r=>setTimeout(r,10));if(count!==2)throw new Error('both routes must be held');await context.close();for(let i=0;i<100;i++)__neo_runtime.get(JSON.stringify({guid:'context@default',field:'pages'}));await new Promise(()=>{});`,
    )
    child = spawn(host, [String(proxy.port), second.targetId, script])
    let stdout = ''
    let stderr = ''
    child.stdout.on('data', (data) => {
      stdout += data
    })
    child.stderr.on('data', (data) => {
      stderr += data
    })
    const exited = once(child, 'exit')
    await eventually(
      () => proxy.held,
      'first real browser continuation reply held',
      12_000,
    )
    await eventually(
      () => proxy.reads >= 90,
      'context.close resolved in the live realm',
      12_000,
    )
    await eventually(
      async () =>
        Object.keys(await read(second.sessionId, 'window.finished')).length > 0,
      'at least first request finished',
    )
    const before = {
      finished: await read(second.sessionId, 'window.finished'),
      continues: proxy.continues,
      disables: proxy.disables,
    }
    proxy.release()
    await new Promise((resolve) => setTimeout(resolve, 200))
    const afterReply = {
      finished: await read(second.sessionId, 'window.finished'),
      continues: proxy.continues,
      disables: proxy.disables,
    }
    child.stdin.write('stop\n')
    const [code] = await exited
    const afterEnd = await read(second.sessionId, 'window.finished')
    t.diagnostic(JSON.stringify({ reference, before, afterReply, afterEnd }))
    assert.equal(code, 0, `${stderr}\n${stdout}`)
    assert.deepEqual(
      before.finished,
      reference,
      'all held routes released before successful close',
    )
    assert.ok(
      before.disables > 0,
      'interception disabled before successful close',
    )
    assert.deepEqual(
      afterReply.finished,
      reference,
      'late reply does not leave a route abandoned',
    )
  } finally {
    proxy?.release()
    await pw?.close().catch(() => {})
    if (child && child.exitCode === null) {
      child.stdin.write('stop\n')
      await once(child, 'exit')
    }
    await runCleanups(cleanup)
  }
})
