// Correctness check: a timed-out waiter must stop its predicate after a delayed setup reply settles.
import assert from 'node:assert/strict'
import { spawn } from 'node:child_process'
import { once } from 'node:events'
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import http from 'node:http'
import { createRequire } from 'node:module'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'
import { test } from 'node:test'
import {
  browserBinary,
  buildNeoRun,
  connectCdp,
  connectReference,
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
let root = process.env.NEO_REVIEW_OUTPUT_DIR
const source = `let error;try{await page.waitForFunction(()=>{window.__polls++;return false},undefined,{timeout:1500,polling:5})}catch(e){error=e.name}
await new Promise(resolve=>setTimeout(resolve,5000));const before=await page.evaluate(()=>window.__polls);await new Promise(resolve=>setTimeout(resolve,150));const after=await page.evaluate(()=>window.__polls);return {error,ran:before>0,stopped:after===before}`
test('waitForFunction cleans up after a delayed setup reply', {
  skip,
  timeout: 180000,
}, async (t) => {
  const cleanups = []
  let rule = null
  const wire = []
  if (!root) {
    root = mkdtempSync(join(tmpdir(), 'neo-wait-cleanup-review-'))
    cleanups.push(() => rmSync(root, { recursive: true, force: true }))
  }
  mkdirSync(root, { recursive: true })
  try {
    const bin = buildNeoRun(),
      origin = await serveFixture(cleanups, {
        pages: {
          '/case':
            '<!doctype html><p>wait</p><script>window.__polls=0</script>',
        },
      })
    const browser = await launchBrowser(cleanups, browserBinary),
      cdp = await connectCdp(cleanups, browser.port)
    const version = await (
      await fetch(`http://127.0.0.1:${browser.port}/json/version`)
    ).json()
    const wss = new WebSocketServer({ noServer: true }),
      server = http.createServer(async (req, res) => {
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
          wire.push({ direction: 'send', ...msg })
          if (up.readyState === WebSocket.OPEN) up.send(raw.toString())
          else waiting.push(raw.toString())
        })
        up.on('open', () => {
          for (const raw of waiting.splice(0)) up.send(raw)
        })
        up.on('message', (raw) => {
          const msg = JSON.parse(raw.toString()),
            call = calls.get(msg.id)
          wire.push({ direction: 'receive', ...msg })
          const forward = () => {
            if (down.readyState === WebSocket.OPEN) down.send(raw.toString())
          }
          if (
            rule?.enabled &&
            !rule.held &&
            call?.method === 'Runtime.callFunctionOn' &&
            JSON.stringify(call.params).includes('evaledExpression') &&
            JSON.stringify(call.params).includes('aborted')
          ) {
            rule.held = true
            setTimeout(forward, 3000)
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
    const playwright = await connectReference(server.address().port)
    cleanups.push(() => playwright.close())
    const context = playwright.contexts()[0]
    for (const enabled of [false, true])
      await t.test(enabled ? 'delayed setup' : 'ordinary setup', async () => {
        rule = { enabled, held: false }
        let start = wire.length
        const page = await context.newPage()
        await page.goto(origin + '/case')
        let reference
        try {
          const AsyncFunction = (async () => {}).constructor
          reference = await new AsyncFunction('page', source)(page)
        } finally {
          await page.close()
        }
        const referenceHeld = rule.held
        writeFileSync(
          join(root, `${enabled}-reference-wire.json`),
          JSON.stringify(wire.slice(start), null, 2),
        )
        rule = { enabled, held: false }
        start = wire.length
        const { targetId } = await openTab(cdp, origin + '/case')
        const path = join(root, `${enabled}.js`)
        writeFileSync(path, source)
        let actual
        try {
          const child = spawn(bin, [
            '--cdp-port',
            String(server.address().port),
            '--page',
            targetId,
            '--timeout-ms',
            '30000',
            path,
          ])
          let out = '',
            err = ''
          child.stdout.on('data', (d) => (out += d))
          child.stderr.on('data', (d) => (err += d))
          await once(child, 'exit')
          const outcome = JSON.parse(out)
          assert.equal(outcome.status, 'ok', JSON.stringify(outcome))
          actual = outcome.value
        } finally {
          await cdp.send('Target.closeTarget', { targetId })
        }
        writeFileSync(
          join(root, `${enabled}-neo-wire.json`),
          JSON.stringify(wire.slice(start), null, 2),
        )
        t.diagnostic(
          JSON.stringify({
            enabled,
            referenceHeld,
            neoHeld: rule.held,
            reference,
            actual,
          }),
        )
        assert.equal(
          referenceHeld,
          enabled,
          'the reference setup reply was delayed as requested',
        )
        assert.equal(
          rule.held,
          enabled,
          'the Neo setup reply was delayed as requested',
        )
        assert.deepEqual(reference, {
          error: 'TimeoutError',
          ran: true,
          stopped: true,
        })
        assert.deepEqual(
          actual,
          reference,
          'the page predicate stops after the timed-out setup settles',
        )
      })
  } finally {
    await runCleanups(cleanups)
  }
})
