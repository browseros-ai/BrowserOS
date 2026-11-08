// Neo: navigation must release an old document's storage turn before its reply arrives.
import assert from 'node:assert/strict'
import { once } from 'node:events'
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import http from 'node:http'
import { createRequire } from 'node:module'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'
import { test } from 'node:test'
import {
  browserBinary,
  buildNeoRun,
  chromium,
  connectCdp,
  launchBrowser,
  openTab,
  runCleanups,
  runInNeo,
  runInPlaywright,
  serveFixture,
  skip,
  waitForUrl,
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
async function until(fn, why, limit = 20000) {
  const end = Date.now() + limit
  while (!(await fn())) {
    assert.ok(Date.now() < end, why)
    await sleep(20)
  }
}
const source = `let firstState='pending'
page.localStorage.setItem('held','first').then(()=>{firstState='ok'},()=>{firstState='error'})
await page.evaluate(()=>{window.before=true})
await page.waitForURL('**/after')
await page.sessionStorage.setItem('after','ok')
await page.evaluate(()=>{window.afterDone=true})
return {value:await page.sessionStorage.getItem('after'),firstState}
`

test('new-document storage remains live while an old-document reply is delayed', {
  skip,
  timeout: 240000,
}, async (t) => {
  const folder = mkdtempSync(join(tmpdir(), 'w1-nav-review-'))
  const results = {}
  const binary = buildNeoRun()
  const file = join(folder, 'agent.js')
  writeFileSync(file, source)
  try {
    for (const engine of ['playwright', 'neo']) {
      const cleanups = []
      let release, job
      try {
        const origin = await serveFixture(cleanups, {
          pages: {
            '/case': '<title>before</title>',
            '/after': '<title>after</title>',
          },
        })
        const browser = await launchBrowser(cleanups, browserBinary)
        const cdp = await connectCdp(cleanups, browser.port)
        const { targetId, sessionId } = await openTab(cdp, `${origin}/case`)
        const evaluate = async (expression) =>
          (
            await cdp.send(
              'Runtime.evaluate',
              { expression, returnByValue: true },
              sessionId,
            )
          ).result.value
        const version = await (
          await fetch(`http://127.0.0.1:${browser.port}/json/version`)
        ).json()
        const wss = new WebSocketServer({ noServer: true })
        const server = http.createServer(async (req, res) => {
          const data = await (
            await fetch(`http://127.0.0.1:${browser.port}${req.url}`)
          ).json()
          if (data.webSocketDebuggerUrl)
            data.webSocketDebuggerUrl = `ws://127.0.0.1:${server.address().port}/browser`
          res.writeHead(200, { 'content-type': 'application/json' })
          res.end(JSON.stringify(data))
        })
        let held = false
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
                if (down.readyState === WebSocket.OPEN)
                  down.send(raw.toString())
              }
              const p = call?.params
              const neo =
                call?.method === 'Runtime.callFunctionOn' &&
                p?.arguments?.[0]?.value === 'webStorageSetItem' &&
                p?.arguments?.[1]?.value?.name === 'held'
              const pw =
                ['Runtime.evaluate', 'Runtime.callFunctionOn'].includes(
                  call?.method,
                ) &&
                JSON.stringify(p).includes('localStorage.setItem') &&
                JSON.stringify(p).includes('held')
              // Delay delivery after the browser executed the call, leaving lifecycle events live.
              if (!held && (engine === 'neo' ? neo : pw)) {
                assert.equal(msg.error, undefined)
                held = true
                release = forward
                t.diagnostic(`Held completed ${engine} storage response`)
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
        const port = server.address().port
        if (engine === 'playwright') {
          const pw = await chromium.connectOverCDP(`http://127.0.0.1:${port}`)
          cleanups.push(() => pw.close())
          job = runInPlaywright(pw.contexts()[0], targetId, source)
        } else job = runInNeo(binary, port, targetId, file)
        let finished = false
        job.then(
          () => {
            finished = true
          },
          () => {
            finished = true
          },
        )
        await until(() => held, 'storage response was held')
        await until(
          async () => await evaluate('window.before===true'),
          'unrelated evaluation still ran',
        )
        assert.equal(await evaluate('localStorage.getItem("held")'), 'first')
        const url = `${origin.replace('127.0.0.1', 'localhost')}/after`
        await cdp.send('Page.navigate', { url }, sessionId)
        await waitForUrl(cdp, sessionId, url)
        // The bound detects a stuck script; only completion and browser state are compared.
        const end = Date.now() + 15000
        while (!finished && Date.now() < end) await sleep(20)
        const beforeRelease = {
          finished,
          state: await evaluate(
            '({host:location.hostname,value:sessionStorage.getItem("after"),done:window.afterDone===true})',
          ),
        }
        release()
        release = null
        const outcome = await job
        results[engine] = {
          beforeRelease,
          value: outcome.value,
          error: outcome.error,
        }
        t.diagnostic(JSON.stringify({ engine, ...results[engine] }))
      } finally {
        if (release) release()
        if (job) await job.catch(() => {})
        await runCleanups(cleanups)
      }
    }
    assert.equal(results.playwright.beforeRelease.finished, true)
    assert.equal(results.playwright.beforeRelease.state.value, 'ok')
    assert.equal(
      results.neo.beforeRelease.finished,
      results.playwright.beforeRelease.finished,
    )
    assert.deepEqual(
      results.neo.beforeRelease.state,
      results.playwright.beforeRelease.state,
    )
  } finally {
    rmSync(folder, { recursive: true, force: true })
  }
})
