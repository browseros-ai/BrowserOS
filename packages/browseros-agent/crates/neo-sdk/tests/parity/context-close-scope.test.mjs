// After an awaited `context.close()` in a run whose realm goes on, the run's init scripts are
// gone from the user's tab: its next document runs none of them, as none survives Playwright's
// close of a connectOverCDP default context (a disconnect; the tab stays). Before D1, Neo's
// close ended the run's calls and subscriptions but left its init scripts installed until the
// run itself ended.
//
// Playwright runs first in the shared browser; Neo then runs through `context-close-host.rs`,
// which keeps the run alive until the driver says stop, behind a CDP proxy that counts the
// `Browser.getTabs` reads of the script's post-close getter loop: once they arrive, the close
// has resolved and the realm is still live, so the driver navigates the tab and reads the
// script's marker.
//
// From the batch-2 delta review (review-delta-8d74c805a, D1, the init-script half of its
// `context-close-scope` repro).
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

/** The global the run's init script sets. */
const MARKER = 'closeScopeInit'
/** The script's sync getter reads after its close, and how many the driver waits for. */
const READS = 200
const READ = 150

/** Builds `context-close-host.rs` against the crates `neo_run` is built from. */
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
    { encoding: 'utf8' },
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
  for (const name of ['browseros_cdp', 'neo_runtime', 'tokio', 'tokio_util']) {
    assert.ok(libs.has(name), `missing ${name} library`)
    args.push('--extern', `${name}=${libs.get(name)}`)
  }
  const bin = join(root, 'context-close-host')
  args.push(
    fileURLToPath(new URL('./context-close-host.rs', import.meta.url)),
    '-o',
    bin,
  )
  const compiled = spawnSync('rustc', args, { encoding: 'utf8' })
  assert.equal(compiled.status, 0, compiled.stderr)
  return bin
}

/**
 * A CDP proxy in front of `upstream` that counts the `Browser.getTabs` requests sent after the
 * run's init script went in (`Page.addScriptToEvaluateOnNewDocument` with the marker).
 */
async function countingProxy(cleanups, upstream) {
  let port
  let armed = false
  let reads = 0
  const sockets = new Set()
  const server = http.createServer((_request, response) =>
    response.end(
      JSON.stringify({ webSocketDebuggerUrl: `ws://127.0.0.1:${port}/proxy` }),
    ),
  )
  const wss = new WebSocketServer({ server })
  wss.on('connection', (down) => {
    const up = new WebSocket(upstream)
    const queue = []
    sockets.add(down)
    sockets.add(up)
    up.on('open', () => {
      for (const data of queue) up.send(data)
      queue.length = 0
    })
    down.on('message', (data) => {
      const text = data.toString()
      const message = JSON.parse(text)
      if (
        message.method === 'Page.addScriptToEvaluateOnNewDocument' &&
        String(message.params?.source).includes(MARKER)
      )
        armed = true
      if (armed && message.method === 'Browser.getTabs') reads++
      if (up.readyState === WebSocket.OPEN) up.send(text)
      else queue.push(text)
    })
    up.on('message', (data) => {
      if (down.readyState === WebSocket.OPEN) down.send(data.toString())
    })
    down.on('close', () => up.close())
    up.on('close', () => down.close())
    down.on('error', () => {})
    up.on('error', () => {})
  })
  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve))
  port = server.address().port
  cleanups.push(async () => {
    for (const socket of sockets) socket.close()
    await new Promise((resolve) => wss.close(resolve))
    await new Promise((resolve) => server.close(resolve))
  })
  return {
    port,
    get reads() {
      return reads
    },
  }
}

test('context.close releases the run’s init scripts while the realm goes on', {
  skip,
  timeout: 180_000,
}, async (t) => {
  const root = mkdtempSync(join(tmpdir(), 'neo-context-close-'))
  const cleanups = [() => rmSync(root, { recursive: true, force: true })]
  let child
  try {
    const bin = buildHost(root)
    const origin = await serveFixture(cleanups, {
      pages: {
        '/top.html': '<!doctype html><title>top</title><p>Old</p>',
        '/after.html': '<!doctype html><title>after</title><p>After</p>',
      },
    })
    const { port } = await launchBrowser(cleanups, browserBinary)
    const cdp = await connectCdp(cleanups, port)
    const read = async (session, expression) =>
      (
        await cdp.send(
          'Runtime.evaluate',
          { expression, returnByValue: true },
          session,
        )
      ).result.value
    /** The marker on the tab's next document. */
    const afterNavigation = async (session) => {
      await cdp.send('Page.navigate', { url: `${origin}/after.html` }, session)
      await eventually(
        async () =>
          (await read(
            session,
            'document.readyState === "complete" && location.pathname',
          )) === '/after.html',
        'the tab loaded /after.html',
      )
      return await read(session, `window.${MARKER} ?? null`)
    }

    const first = await openTab(cdp, `${origin}/top.html`)
    const playwright = await chromium.connectOverCDP(`http://127.0.0.1:${port}`)
    const [context] = playwright.contexts()
    await context.addInitScript(`window.${MARKER} = 19`)
    await context.close()
    const reference = await afterNavigation(first.sessionId)
    await playwright.close()
    await cdp.send('Target.closeTarget', { targetId: first.targetId })

    const second = await openTab(cdp, `${origin}/top.html`)
    const upstream = (
      await (await fetch(`http://127.0.0.1:${port}/json/version`)).json()
    ).webSocketDebuggerUrl
    const proxy = await countingProxy(cleanups, upstream)
    const script = join(root, 'context-close-scope.js')
    writeFileSync(
      script,
      `await context.addInitScript(() => { window.${MARKER} = 19 })
await context.close()
for (let i = 0; i < ${READS}; i++)
  __neo_runtime.get(JSON.stringify({ guid: 'context@default', field: 'pages' }))
await new Promise(() => {})`,
    )
    child = spawn(bin, [String(proxy.port), second.targetId, script])
    let stdout = ''
    let stderr = ''
    child.stdout.on('data', (chunk) => {
      stdout += chunk
    })
    child.stderr.on('data', (chunk) => {
      stderr += chunk
    })
    const exited = once(child, 'exit')
    await eventually(
      () => proxy.reads >= READ,
      'the realm read the context after its close',
      30_000,
    )
    const neo = await afterNavigation(second.sessionId)
    child.stdin.write('stop\n')
    const [code] = await exited
    t.diagnostic(
      JSON.stringify({ playwright: reference, neo, run: stdout.trim() }),
    )
    assert.equal(code, 0, stderr)
    assert.equal(reference, null, 'Playwright’s close leaves no init script')
    assert.equal(neo, reference, 'Neo’s close leaves no init script')
  } finally {
    if (child && child.exitCode === null) {
      child.kill('SIGTERM')
      await once(child, 'exit')
    }
    await runCleanups(cleanups)
  }
})
