// A renderer crash must not leave a browser-owned PDF stream open.
import assert from 'node:assert/strict'
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
  launchBrowser,
  openTab,
  runCleanups,
  serveFixture,
  skip,
  startNeo,
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
async function fixtureWait(promise, label) {
  let timer
  try {
    return await Promise.race([
      promise,
      new Promise((_, reject) => {
        timer = setTimeout(() => reject(new Error(label)), 30000)
      }),
    ])
  } finally {
    clearTimeout(timer)
  }
}

test('a crashed tab closes an abandoned PDF stream', {
  skip,
  timeout: 180000,
}, async (t) => {
  const root =
    process.env.NEO_REVIEW_OUTPUT_DIR ??
    mkdtempSync(join(tmpdir(), 'neo-pdf-crash-'))
  mkdirSync(root, { recursive: true })
  const cleanups = []
  if (!process.env.NEO_REVIEW_OUTPUT_DIR)
    cleanups.push(() => rmSync(root, { recursive: true, force: true }))
  let stream
  let releaseReply
  let heldResolve
  const held = new Promise((resolve) => {
    heldResolve = resolve
  })
  try {
    const bin = buildNeoRun()
    const origin = await serveFixture(cleanups, {
      pages: { '/case': '<!doctype html><p>PDF stream cleanup</p>' },
    })
    const browser = await launchBrowser(cleanups, browserBinary)
    const cdp = await connectCdp(cleanups, browser.port)
    const { targetId, sessionId } = await openTab(cdp, `${origin}/case`)
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
        const waiting = []
        const calls = new Map()
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
          const reply = JSON.parse(raw.toString())
          const call = calls.get(reply.id)
          const forward = () => {
            if (down.readyState === WebSocket.OPEN) down.send(raw.toString())
          }
          if (
            !stream &&
            call?.method === 'Page.printToPDF' &&
            reply.result?.stream
          ) {
            stream = reply.result.stream
            releaseReply = forward
            heldResolve()
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
          for (const client of wss.clients) client.terminate()
          wss.close()
          server.close(resolve)
        }),
    )
    // The controller keeps its own debugger session, as another browser client may.
    cleanups.push(async () => {
      releaseReply?.()
      if (stream)
        await cdp
          .send('IO.close', { handle: stream }, sessionId)
          .catch(() => {})
      await cdp.send('Target.closeTarget', { targetId }).catch(() => {})
    })
    const script = join(root, 'pdf-crash.js')
    writeFileSync(
      script,
      `try { await page.pdf(); return {captured:true} } catch(error) { return {closed:error.name==='Error' && /closed|crashed/i.test(error.message)} }`,
    )
    const neo = startNeo(bin, server.address().port, targetId, script)
    await neo.connected
    await fixtureWait(held, 'the browser created its PDF stream')
    const before = await cdp.send(
      'IO.read',
      { handle: stream, offset: 0, size: 8 },
      sessionId,
    )
    assert.equal(
      typeof before.data,
      'string',
      'the real PDF stream exists before the crash',
    )
    void cdp.send('Page.crash', {}, sessionId).catch(() => {})
    // Keep the unchanged answer delayed across the crash, as the existing cleanup review does.
    const forwarding = sleep(6500).then(() => releaseReply())
    const outcome = await fixtureWait(
      neo.outcome,
      'the crashed run and cleanup settled',
    )
    await forwarding
    assert.equal(outcome.status, 'ok', JSON.stringify(outcome))
    assert.equal(outcome.value.closed, true, JSON.stringify(outcome))
    let accessible = false
    try {
      const reply = await cdp.send(
        'IO.read',
        { handle: stream, offset: 0, size: 8 },
        sessionId,
      )
      accessible = typeof reply.data === 'string'
    } catch {}
    t.diagnostic(
      JSON.stringify({
        status: outcome.status,
        value: outcome.value,
        streamAccessible: accessible,
      }),
    )
    writeFileSync(
      join(root, 'result.json'),
      JSON.stringify({ outcome, streamAccessible: accessible }, null, 2),
    )
    assert.equal(accessible, false, 'the abandoned PDF stream was closed')
  } finally {
    await runCleanups(cleanups)
  }
})
