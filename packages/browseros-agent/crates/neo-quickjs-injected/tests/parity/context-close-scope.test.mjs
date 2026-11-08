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
// The second case holds the browser's reply to the script's installation, so the close comes
// while that installation is still out: the tab's next document runs no script, before or
// after the reply arrives, as Playwright's control of the same delay reads none. The third
// exposes a function of the context: after the close, the tab's next document has neither the
// function nor the CDP binding it calls through.
//
// From the batch-2 delta review (review-delta-8d74c805a, D1, the init-script half of its
// `context-close-scope` repro) and the D1 re-review's item 2 (`late-init`).
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
 * run's init script went in (`Page.addScriptToEvaluateOnNewDocument` whose source holds `arm`).
 * With `hold`, it keeps the browser's reply to that installation until `release()`.
 */
async function countingProxy(
  cleanups,
  upstream,
  { hold = false, arm = MARKER } = {},
) {
  let port
  let armed = false
  let reads = 0
  let held = null
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
    const installs = new Set()
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
        String(message.params?.source).includes(arm)
      ) {
        armed = true
        installs.add(message.id)
      }
      if (armed && message.method === 'Browser.getTabs') reads++
      if (up.readyState === WebSocket.OPEN) up.send(text)
      else queue.push(text)
    })
    up.on('message', (data) => {
      const text = data.toString()
      const reply = () => {
        if (down.readyState === WebSocket.OPEN) down.send(text)
      }
      if (hold && !held && installs.has(JSON.parse(text).id)) held = reply
      else reply()
    })
    down.on('close', () => up.close())
    up.on('close', () => down.close())
    down.on('error', () => {})
    up.on('error', () => {})
  })
  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve))
  port = server.address().port
  const release = () => {
    const reply = held
    held = () => {}
    reply?.()
  }
  cleanups.push(async () => {
    release()
    for (const socket of sockets) socket.close()
    await new Promise((resolve) => wss.close(resolve))
    await new Promise((resolve) => server.close(resolve))
  })
  return {
    port,
    get reads() {
      return reads
    },
    get held() {
      return held !== null
    },
    release,
  }
}

/** The tab's marker on its next document, loaded from `path`. */
async function afterNavigation(cdp, session, origin, path) {
  const read = async (expression) =>
    (
      await cdp.send(
        'Runtime.evaluate',
        { expression, returnByValue: true },
        session,
      )
    ).result.value
  await cdp.send('Page.navigate', { url: `${origin}${path}` }, session)
  await eventually(
    async () =>
      (await read(
        `document.readyState === "complete" && location.pathname`,
      )) === path,
    `the tab loaded ${path}`,
  )
  return await read(`window.${MARKER} ?? null`)
}

/** Starts the host on `targetId` behind `proxy`, with its output kept. */
function startHost(bin, proxy, targetId, script) {
  const child = spawn(bin, [String(proxy.port), targetId, script])
  const host = { child, stdout: '', stderr: '', exited: once(child, 'exit') }
  child.stdout.on('data', (chunk) => {
    host.stdout += chunk
  })
  child.stderr.on('data', (chunk) => {
    host.stderr += chunk
  })
  return host
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

test('an init-script installation still out at context.close leaves no script', {
  skip,
  timeout: 180_000,
}, async (t) => {
  const root = mkdtempSync(join(tmpdir(), 'neo-context-close-late-'))
  const cleanups = [() => rmSync(root, { recursive: true, force: true })]
  let host
  try {
    const bin = buildHost(root)
    const origin = await serveFixture(cleanups, {
      pages: {
        '/top.html': '<!doctype html><title>top</title>',
        '/after-close.html': '<!doctype html><title>after close</title>',
        '/after-reply.html': '<!doctype html><title>after reply</title>',
      },
    })
    const { port } = await launchBrowser(cleanups, browserBinary)
    const cdp = await connectCdp(cleanups, port)
    const tab = await openTab(cdp, `${origin}/top.html`)
    const read = async (expression) =>
      (
        await cdp.send(
          'Runtime.evaluate',
          { expression, returnByValue: true },
          tab.sessionId,
        )
      ).result.value
    const upstream = (
      await (await fetch(`http://127.0.0.1:${port}/json/version`)).json()
    ).webSocketDebuggerUrl
    const proxy = await countingProxy(cleanups, upstream, { hold: true })
    const script = join(root, 'context-close-late.js')
    // The installation starts unawaited; the driver lets the close go once its reply is held.
    writeFileSync(
      script,
      `const closeGate = page.evaluate(() => new Promise((resolve) => { window.closeNow = resolve }))
await page.evaluate(() => true)
context.addInitScript(() => { window.${MARKER} = 19 }).catch(() => {})
await closeGate
await context.close()
for (let i = 0; i < ${READS}; i++)
  __neo_runtime.get(JSON.stringify({ guid: 'context@default', field: 'pages' }))
await new Promise(() => {})`,
    )
    host = startHost(bin, proxy, tab.targetId, script)
    await eventually(
      () => proxy.held,
      'the installation’s reply is held',
      20_000,
    )
    await eventually(
      async () => await read("typeof window.closeNow === 'function'"),
      'the script waits to close',
    )
    await read('window.closeNow()')
    await eventually(
      () => proxy.reads >= READ,
      'the realm read the context after its close',
      30_000,
    )
    const beforeReply = await afterNavigation(
      cdp,
      tab.sessionId,
      origin,
      '/after-close.html',
    )
    proxy.release()
    await new Promise((resolve) => setTimeout(resolve, 200))
    const afterReply = await afterNavigation(
      cdp,
      tab.sessionId,
      origin,
      '/after-reply.html',
    )
    host.child.stdin.write('stop\n')
    const [code] = await host.exited
    t.diagnostic(
      JSON.stringify({ beforeReply, afterReply, run: host.stdout.trim() }),
    )
    assert.equal(code, 0, host.stderr)
    assert.equal(beforeReply, null, 'no script runs after the awaited close')
    assert.equal(afterReply, null, 'the late reply installs nothing')
  } finally {
    if (host && host.child.exitCode === null) {
      host.child.kill('SIGTERM')
      await host.exited
    }
    await runCleanups(cleanups)
  }
})

test('context.close releases the run’s exposed bindings while the realm goes on', {
  skip,
  timeout: 180_000,
}, async (t) => {
  const root = mkdtempSync(join(tmpdir(), 'neo-context-close-binding-'))
  const cleanups = [() => rmSync(root, { recursive: true, force: true })]
  let host
  try {
    const bin = buildHost(root)
    const origin = await serveFixture(cleanups, {
      pages: {
        '/top.html': '<!doctype html><title>top</title>',
        '/after.html': '<!doctype html><title>after</title>',
      },
    })
    const { port } = await launchBrowser(cleanups, browserBinary)
    const cdp = await connectCdp(cleanups, port)
    const binding = async (session) => {
      await cdp.send('Page.navigate', { url: `${origin}/after.html` }, session)
      const read = async (expression) =>
        (
          await cdp.send(
            'Runtime.evaluate',
            { expression, returnByValue: true },
            session,
          )
        ).result.value
      await eventually(
        async () =>
          (await read(
            'document.readyState === "complete" && location.pathname',
          )) === '/after.html',
        'the tab loaded /after.html',
      )
      // The function, and the CDP binding it sends through (`__playwright__binding__`), which
      // only the binding's own release takes off the session.
      return await read(
        '({ binding: typeof window.closedFn, transport: typeof window.__playwright__binding__ })',
      )
    }

    const first = await openTab(cdp, `${origin}/top.html`)
    const playwright = await chromium.connectOverCDP(`http://127.0.0.1:${port}`)
    const [context] = playwright.contexts()
    await context.exposeFunction('closedFn', () => 41)
    await context.close()
    const reference = await binding(first.sessionId)
    await playwright.close()
    await cdp.send('Target.closeTarget', { targetId: first.targetId })

    const second = await openTab(cdp, `${origin}/top.html`)
    const upstream = (
      await (await fetch(`http://127.0.0.1:${port}/json/version`)).json()
    ).webSocketDebuggerUrl
    const proxy = await countingProxy(cleanups, upstream, { arm: 'closedFn' })
    const script = join(root, 'context-close-binding.js')
    writeFileSync(
      script,
      `await context.exposeFunction('closedFn', () => 41)
await context.close()
for (let i = 0; i < ${READS}; i++)
  __neo_runtime.get(JSON.stringify({ guid: 'context@default', field: 'pages' }))
await new Promise(() => {})`,
    )
    host = startHost(bin, proxy, second.targetId, script)
    await eventually(
      () => proxy.reads >= READ,
      'the realm read the context after its close',
      30_000,
    )
    const neo = await binding(second.sessionId)
    host.child.stdin.write('stop\n')
    const [code] = await host.exited
    t.diagnostic(
      JSON.stringify({ playwright: reference, neo, run: host.stdout.trim() }),
    )
    assert.equal(code, 0, host.stderr)
    assert.deepEqual(
      reference,
      { binding: 'undefined', transport: 'undefined' },
      'Playwright’s close leaves no binding',
    )
    assert.deepEqual(neo, reference, 'Neo’s close leaves no binding')
  } finally {
    if (host && host.child.exitCode === null) {
      host.child.kill('SIGTERM')
      await host.exited
    }
    await runCleanups(cleanups)
  }
})
