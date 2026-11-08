// A cancelled screenshot must release its background before the next run captures the tab.
import assert from 'node:assert/strict'
import { spawn, spawnSync } from 'node:child_process'
import { once } from 'node:events'
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs'
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
  const bin = join(root, 'capture-next-run-host')
  args.push(
    fileURLToPath(new URL('./capture-next-run-host.rs', import.meta.url)),
    '-o',
    bin,
  )
  const compiled = spawnSync('rustc', args, { encoding: 'utf8' })
  assert.equal(compiled.status, 0, compiled.stderr)
  return bin
}

test('a new run captures after cancelled screenshot background setup', {
  skip,
  timeout: 240000,
}, async (t) => {
  const cleanups = []
  const root =
    process.env.NEO_REVIEW_OUTPUT_DIR ??
    mkdtempSync(join(tmpdir(), 'neo-capture-next-run-'))
  mkdirSync(root, { recursive: true })
  if (!process.env.NEO_REVIEW_OUTPUT_DIR)
    cleanups.push(() => rmSync(root, { recursive: true, force: true }))
  let rule = null
  try {
    const bin = buildHost(root)
    const origin = await serveFixture(cleanups, {
      pages: {
        '/case':
          '<!doctype html><input style="caret-color:red"><div id=box style="height:40px;width:50px">box</div>',
      },
    })
    const browser = await launchBrowser(cleanups, browserBinary)
    const cdp = await connectCdp(cleanups, browser.port)
    const version = await (
      await fetch(`http://127.0.0.1:${browser.port}/json/version`)
    ).json()
    const wss = new WebSocketServer({ noServer: true })
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
        const up = new WebSocket(version.webSocketDebuggerUrl)
        const waiting = [],
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
            call &&
            (call.method === rule.method ||
              (call.method === 'Runtime.callFunctionOn' &&
                call.params.arguments?.[0]?.value === rule.method))
          ) {
            rule.held = true
            writeFileSync(rule.marker, 'held')
            setTimeout(forward, 6500)
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
    for (const c of [
      {
        name: 'preparing control',
        method: 'prepareForScreenshot',
        options: { style: 'input{opacity:.5}' },
        drop: false,
      },
      {
        name: 'cancel background then capture in next run',
        method: 'Emulation.setDefaultBackgroundColorOverride',
        options: { omitBackground: true },
        drop: false,
      },
      {
        name: 'drop background then capture in next run',
        method: 'Emulation.setDefaultBackgroundColorOverride',
        options: { omitBackground: true },
        drop: true,
      },
    ])
      await t.test(c.name, async () => {
        const { targetId } = await openTab(cdp, origin + '/case')
        const marker = join(root, `${targetId}.held`)
        rule = { method: c.method, marker, held: false }
        try {
          const child = spawn(bin, ['--nocapture'], {
            env: {
              ...process.env,
              NEO_REVIEW_CDP_PORT: String(server.address().port),
              NEO_REVIEW_TARGET: targetId,
              NEO_REVIEW_MARKER: marker,
              NEO_REVIEW_CODE: `await page.screenshot(${JSON.stringify({ ...c.options, timeout: 0 })})`,
              NEO_REVIEW_DROP: c.drop ? '1' : '0',
            },
          })
          let out = '',
            err = ''
          child.stdout.on('data', (d) => {
            out += d
          })
          child.stderr.on('data', (d) => {
            err += d
          })
          const [exit] = await once(child, 'exit')
          t.diagnostic(
            JSON.stringify({ name: c.name, held: rule.held, out, err }),
          )
          assert.ok(
            rule.held,
            'the browser setup reply was delayed after its effect',
          )
          assert.equal(
            exit,
            0,
            'the next run captures with the default opaque background',
          )
        } finally {
          rule = null
          await cdp.send('Target.closeTarget', { targetId })
        }
      })
  } finally {
    await runCleanups(cleanups)
  }
})
