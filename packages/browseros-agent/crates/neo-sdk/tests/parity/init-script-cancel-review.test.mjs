// A cancelled init-script setup must leave the next document unchanged.
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

const sleep = (ms) => new Promise((r) => setTimeout(r, ms))
async function waitFor(fn) {
  const end = Date.now() + 20000
  while (!(await fn())) {
    assert.ok(Date.now() < end, 'the fixture reached the required state')
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
  const bin = join(root, 'init-script-cancel-host')
  args.push(
    fileURLToPath(new URL('./init-script-cancel-host.rs', import.meta.url)),
    '-o',
    bin,
  )
  const compiled = spawnSync('rustc', args, { encoding: 'utf8' })
  assert.equal(compiled.status, 0, compiled.stderr)
  return bin
}

test('init scripts are absent after cancelling a setup whose reply is delayed', {
  skip,
  timeout: 180000,
}, async (t) => {
  const root =
    process.env.NEO_REVIEW_OUTPUT_DIR ??
    mkdtempSync(join(tmpdir(), 'neo-init-script-cancel-'))
  mkdirSync(root, { recursive: true })
  const cleanups = []
  if (!process.env.NEO_REVIEW_OUTPUT_DIR)
    cleanups.push(() => rmSync(root, { recursive: true, force: true }))
  let rule = null
  const bin = buildHost(root)
  try {
    const origin = await serveFixture(cleanups, {
      pages: {
        '/case': '<!doctype html><p>case</p>',
        '/after':
          '<!doctype html><script>window.sawOldRun=window.oldRunInit??null;window.pageReady=true</script><p>after</p>',
      },
    })
    const browser = await launchBrowser(cleanups, browserBinary),
      cdp = await connectCdp(cleanups, browser.port)
    const version = await (
        await fetch(`http://127.0.0.1:${browser.port}/json/version`)
      ).json(),
      wss = new WebSocketServer({ noServer: true })
    const server = http.createServer(async (req, res) => {
      const json = await (
        await fetch(`http://127.0.0.1:${browser.port}${req.url}`)
      ).json()
      if (json.webSocketDebuggerUrl)
        json.webSocketDebuggerUrl = `ws://127.0.0.1:${server.address().port}/browser`
      res.writeHead(200, { 'content-type': 'application/json' })
      res.end(JSON.stringify(json))
    })
    server.on('upgrade', (req, socket, head) =>
      wss.handleUpgrade(req, socket, head, (down) => {
        const up = new WebSocket(version.webSocketDebuggerUrl),
          waiting = [],
          calls = new Map()
        down.on('message', (raw) => {
          const msg = JSON.parse(raw.toString())
          calls.set(msg.id, msg)
          if (up.readyState === WebSocket.OPEN) up.send(raw.toString())
          else waiting.push(raw.toString())
        })
        up.on('open', () => {
          for (const raw of waiting.splice(0)) up.send(raw)
        })
        up.on('message', (raw) => {
          const msg = JSON.parse(raw.toString()),
            call = calls.get(msg.id)
          const forward = () => {
            if (down.readyState === WebSocket.OPEN) down.send(raw.toString())
          }
          if (
            rule &&
            !rule.held &&
            call?.method === 'Target.setAutoAttach' &&
            call.params?.autoAttach === true
          ) {
            rule.held = true
            writeFileSync(rule.marker, 'ready')
            setTimeout(forward, rule.delay)
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
    for (const delayed of [false, true])
      await t.test(delayed ? 'delayed setup' : 'ordinary setup', async () => {
        const { targetId, sessionId } = await openTab(cdp, origin + '/case')
        const held = join(root, targetId + '.held'),
          returned = join(root, targetId + '.returned'),
          release = join(root, targetId + '.release')
        rule = { held: false, marker: held, delay: delayed ? 6500 : 0 }
        const child = spawn(bin, ['--nocapture'], {
          env: {
            ...process.env,
            NEO_REVIEW_CDP_PORT: String(server.address().port),
            NEO_REVIEW_TARGET: targetId,
            NEO_REVIEW_HELD: held,
            NEO_REVIEW_RETURNED: returned,
            NEO_REVIEW_RELEASE: release,
          },
        })
        let out = '',
          err = ''
        child.stdout.on('data', (d) => (out += d))
        child.stderr.on('data', (d) => (err += d))
        const exited = once(child, 'exit')
        let value
        try {
          await waitFor(() => existsSync(returned))
          assert.ok(rule.held, 'the real setup reply was observed')
          await cdp.send('Page.navigate', { url: origin + '/after' }, sessionId)
          await waitFor(async () => {
            try {
              return (
                (
                  await cdp.send(
                    'Runtime.evaluate',
                    {
                      expression:
                        'location.pathname==="/after"&&window.pageReady===true',
                      returnByValue: true,
                    },
                    sessionId,
                  )
                ).result.value === true
              )
            } catch {
              return false
            }
          })
          value = (
            await cdp.send(
              'Runtime.evaluate',
              { expression: 'window.sawOldRun', returnByValue: true },
              sessionId,
            )
          ).result.value
        } finally {
          writeFileSync(release, 'ready')
          const [exit] = await exited
          t.diagnostic(JSON.stringify({ delayed, value, exit, out, err }))
          assert.equal(exit, 0, 'the host cancelled and drained')
          rule = null
          await cdp.send('Target.closeTarget', { targetId })
        }
        assert.equal(
          value,
          null,
          'a document loaded after the run returned has no old init script',
        )
      })
  } finally {
    await runCleanups(cleanups)
  }
})
