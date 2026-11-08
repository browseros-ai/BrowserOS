// A held init-script removal reply must not prevent binding cleanup at context/browser close.
// Ported from the delta review's two held-init-binding repros; the browser applies the removal,
// but this proxy withholds its acknowledgement until after close resolves and the tab is probed.

import assert from 'node:assert/strict'
import { spawn, spawnSync } from 'node:child_process'
import { once } from 'node:events'
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import http from 'node:http'
import { createRequire } from 'node:module'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import {
  browserBinary,
  chromium,
  connectCdp,
  eventually,
  launchBrowser,
  openTab,
  referenceHalf,
  runCleanups,
  serveFixture,
} from './harness.mjs'

/** Reuses the live-after-close host: cancel on stdin, then wait for Runtime::drain. */
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

/** Holds only the first script-removal reply; post-close getters mark the still-live realm. */
async function holdingProxy(cleanups, upstream) {
  const require = createRequire(import.meta.url)
  const { ws: WebSocket, wsServer: WebSocketServer } = require(
    join(
      process.env.NEO_PARITY_PLAYWRIGHT ??
        dirname(require.resolve('playwright-core/package.json')),
      'lib/utilsBundle.js',
    ),
  )
  let port
  let armed = false
  let reads = 0
  let held
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
      if (message.method === 'Runtime.addBinding') {
        armed = true
        reads = 0
      }
      if (armed && message.method === 'Browser.getTabs') reads++
      if (up.readyState === WebSocket.OPEN) up.send(text)
      else queue.push(text)
    })
    up.on('message', (data) => {
      const text = data.toString()
      const reply = JSON.parse(text)
      const call = calls.get(reply.id)
      calls.delete(reply.id)
      if (
        !held &&
        call?.method === 'Page.removeScriptToEvaluateOnNewDocument'
      ) {
        held = () => {
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
  const proxy = {
    port,
    get reads() {
      return reads
    },
    get holding() {
      return Boolean(held)
    },
    release() {
      held?.()
      held = () => {}
    },
  }
  cleanups.push(async () => {
    proxy.release()
    for (const socket of sockets) socket.close()
    await new Promise((resolve) => wss.close(resolve))
    await new Promise((resolve) => server.close(resolve))
  })
  return proxy
}

/** Compares the surviving tab's binding after the same public close in both engines. */
export async function compareHeldInitBinding(t, close) {
  const cleanups = []
  let child
  try {
    const scripts = mkdtempSync(join(tmpdir(), 'neo-held-init-binding-'))
    cleanups.push(() => rmSync(scripts, { recursive: true, force: true }))
    const host = buildHost(scripts)
    const origin = await serveFixture(cleanups, {
      pages: {
        '/top.html': '<!doctype html><p>Old</p>',
        '/after.html': (request, response) => {
          response.writeHead(200, { 'content-type': 'text/html' })
          response.end(
            '<!doctype html><script>window.serverHeader=' +
              JSON.stringify(request.headers['x-review'] ?? null) +
              '</script><p>After</p>',
          )
        },
      },
    })
    const { port } = await launchBrowser(cleanups, browserBinary)
    const cdp = await connectCdp(cleanups, port)
    const observe = async (session) =>
      (
        await cdp.send(
          'Runtime.evaluate',
          {
            // The CDP transport survives navigation unless binding teardown itself ran; removing
            // only the init scripts would hide the exposed function while leaving this installed.
            expression:
              '({binding:typeof window.closedFn,transport:typeof window.__playwright__binding__,header:window.serverHeader})',
            returnByValue: true,
          },
          session,
        )
      ).result.value
    const first = await openTab(cdp, `${origin}/top.html`)
    const reference = await referenceHalf(async () => {
      const pw = await chromium.connectOverCDP(`http://127.0.0.1:${port}`)
      try {
        const page = pw
          .contexts()[0]
          .pages()
          .find((page) => page.url() === `${origin}/top.html`)
        const context = page.context()
        await context.exposeFunction('closedFn', () => 41)
        await context.addInitScript(() => {
          window.decoy = 1
        })
        await new Function('context', `return ${close}`)(context)
        await cdp.send(
          'Page.navigate',
          { url: `${origin}/after.html` },
          first.sessionId,
        )
        await eventually(
          async () => (await observe(first.sessionId)).header !== undefined,
          'reference navigation loaded',
        )
        return await observe(first.sessionId)
      } finally {
        await pw.close()
      }
    })
    await cdp.send('Target.closeTarget', { targetId: first.targetId })
    const second = await openTab(cdp, `${origin}/top.html`)
    const { webSocketDebuggerUrl } = await (
      await fetch(`http://127.0.0.1:${port}/json/version`)
    ).json()
    const proxy = await holdingProxy(cleanups, webSocketDebuggerUrl)
    const script = join(scripts, 'held-init-binding.js')
    writeFileSync(
      script,
      `
await context.exposeFunction('closedFn', () => 41)
await context.addInitScript(() => { window.decoy = 1 })
await ${close}
for (let i = 0; i < 100; i++)
  __neo_runtime.get(JSON.stringify({ guid: 'context@default', field: 'pages' }))
await new Promise(() => {})
`,
    )
    child = spawn(host, [String(proxy.port), second.targetId, script])
    let stderr = ''
    child.stderr.on('data', (data) => {
      stderr += data
    })
    const exited = once(child, 'exit')
    await eventually(
      () => proxy.reads >= 90,
      'post-close getter marker from still-live realm',
      10_000,
    )
    await cdp.send(
      'Page.navigate',
      { url: `${origin}/after.html` },
      second.sessionId,
    )
    await eventually(
      async () => (await observe(second.sessionId)).header !== undefined,
      'Neo navigation loaded',
    )
    const neo = await observe(second.sessionId)
    assert.ok(proxy.holding, 'the init removal reply is held across close')
    proxy.release()
    child.stdin.write('stop\n')
    const [code] = await exited
    t.diagnostic(JSON.stringify({ reference, neo }))
    assert.equal(code, 0, stderr)
    assert.deepEqual(reference, {
      binding: 'undefined',
      transport: 'undefined',
      header: null,
    })
    assert.deepEqual(neo, reference)
  } finally {
    if (child && child.exitCode === null) {
      child.kill('SIGTERM')
      await once(child, 'exit')
    }
    await runCleanups(cleanups)
  }
}
