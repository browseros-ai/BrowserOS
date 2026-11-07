// Correctness checks for progress while another run's unchanged browser reply is held.
import assert from 'node:assert/strict'
import { spawn, spawnSync } from 'node:child_process'
import { once } from 'node:events'
import {
  existsSync,
  mkdirSync,
  mkdtempSync,
  rmSync,
  writeFileSync,
} from 'node:fs'
import http from 'node:http'
import { createRequire } from 'node:module'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'
import { test } from 'node:test'
import { fileURLToPath } from 'node:url'
import {
  browserBinary,
  connectCdp,
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

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))
async function waitFor(fn) {
  const end = Date.now() + 20000
  while (!(await fn())) {
    assert.ok(Date.now() < end, 'fixture reached its required state')
    await sleep(20)
  }
}
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
    '--test',
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
  const bin = join(root, 'capture-turn-liveness-host')
  args.push(
    fileURLToPath(new URL('./capture-turn-liveness-host.rs', import.meta.url)),
    '-o',
    bin,
  )
  const compiled = spawnSync('rustc', args, { encoding: 'utf8' })
  assert.equal(compiled.status, 0, compiled.stderr)
  return bin
}

test('a held screenshot cleanup keeps other calls live', {
  skip,
  timeout: 240000,
}, async (t) => {
  const root =
    process.env.NEO_REVIEW_OUTPUT_DIR ??
    mkdtempSync(join(tmpdir(), 'neo-capture-turn-live-'))
  mkdirSync(root, { recursive: true })
  const cleanups = []
  if (!process.env.NEO_REVIEW_OUTPUT_DIR)
    cleanups.push(() => rmSync(root, { recursive: true, force: true }))
  let rule = null
  try {
    const bin = buildHost(root)
    const origin = await serveFixture(cleanups, {
      pages: { '/case': '<!doctype html><p>turn test</p>' },
    })
    const browser = await launchBrowser(cleanups, browserBinary),
      cdp = await connectCdp(cleanups, browser.port)
    const version = await (
        await fetch(`http://127.0.0.1:${browser.port}/json/version`)
      ).json(),
      wss = new WebSocketServer({ noServer: true })
    const server = http.createServer(async (req, res) => {
      const data = await (
        await fetch(`http://127.0.0.1:${browser.port}${req.url}`)
      ).json()
      if (data.webSocketDebuggerUrl)
        data.webSocketDebuggerUrl = `ws://127.0.0.1:${server.address().port}/browser`
      res.writeHead(200, { 'content-type': 'application/json' })
      res.end(JSON.stringify(data))
    })
    server.on('upgrade', (req, socket, head) =>
      wss.handleUpgrade(req, socket, head, (down) => {
        const up = new WebSocket(version.webSocketDebuggerUrl),
          waiting = [],
          calls = new Map()
        down.on('message', (raw) => {
          const call = JSON.parse(raw.toString())
          calls.set(call.id, call)
          if (up.readyState === WebSocket.OPEN) up.send(raw.toString())
          else waiting.push(raw.toString())
        })
        up.on('open', () => {
          for (const raw of waiting.splice(0)) up.send(raw)
        })
        up.on('message', (raw) => {
          const response = JSON.parse(raw.toString()),
            call = calls.get(response.id),
            forward = () => {
              if (down.readyState === WebSocket.OPEN) down.send(raw.toString())
            }
          if (
            rule &&
            !rule.forward &&
            call?.method === 'Emulation.setDefaultBackgroundColorOverride' &&
            call.params?.color?.a === 0
          ) {
            rule.forward = forward
            writeFileSync(rule.held, 'ready')
          } else forward()
        })
        down.on('close', () => up.close())
        up.on('close', () => down.close())
        up.on('error', () => {})
      }),
    )
    server.listen(0, '127.0.0.1')
    await once(server, 'listening')
    cleanups.push(
      () =>
        new Promise((resolve) => {
          for (const c of wss.clients) c.terminate()
          wss.close()
          server.close(resolve)
        }),
    )
    for (const mode of ['independent', 'closed', 'crashed'])
      await t.test(mode, async () => {
        const main = await openTab(cdp, origin + '/case'),
          other = await openTab(cdp, origin + '/case')
        const held = join(root, main.targetId + '.held'),
          done = join(root, main.targetId + '.done'),
          release = join(root, main.targetId + '.release')
        rule = { held, forward: null }
        const child = spawn(bin, ['--nocapture'], {
          env: {
            ...process.env,
            NEO_REVIEW_CDP_PORT: String(server.address().port),
            NEO_REVIEW_TARGET: main.targetId,
            NEO_REVIEW_OTHER: other.targetId,
            NEO_REVIEW_MODE: mode,
            NEO_REVIEW_HELD: held,
            NEO_REVIEW_DONE: done,
            NEO_REVIEW_RELEASE: release,
          },
        })
        let observedQueued = false,
          actionResult,
          lastProbe
        let out = '',
          err = ''
        child.stdout.on('data', (v) => (out += v))
        child.stderr.on('data', (v) => (err += v))
        const exited = once(child, 'exit')
        try {
          if (mode !== 'independent') {
            await waitFor(async () => {
              assert.equal(child.exitCode, null, out + err)
              try {
                lastProbe = await cdp.send(
                  'Runtime.evaluate',
                  {
                    expression: 'document.documentElement.dataset.turnQueued',
                    returnByValue: true,
                  },
                  main.sessionId,
                )
                return lastProbe.result.value === 'yes'
              } catch (error) {
                lastProbe = { error: error.message }
                return false
              }
            })
            observedQueued = true
            if (mode === 'closed')
              actionResult = await cdp.send('Target.closeTarget', {
                targetId: main.targetId,
              })
            else void cdp.send('Page.crash', {}, main.sessionId).catch(() => {})
          }
          await waitFor(() => {
            assert.equal(child.exitCode, null, out + err)
            return existsSync(done)
          })
          assert.ok(
            rule.forward,
            'the other call settled before the held reply was released',
          )
        } finally {
          rule.forward?.()
          writeFileSync(release, 'ready')
          const [code] = await exited
          t.diagnostic(
            JSON.stringify({
              mode,
              observedQueued,
              actionResult,
              lastProbe,
              code,
              out,
              err,
            }),
          )
          rule = null
          await cdp
            .send('Target.closeTarget', { targetId: main.targetId })
            .catch(() => {})
          await cdp
            .send('Target.closeTarget', { targetId: other.targetId })
            .catch(() => {})
          assert.equal(code, 0, 'host completed both runs and drained cleanup')
        }
      })
  } finally {
    await runCleanups(cleanups)
  }
})
