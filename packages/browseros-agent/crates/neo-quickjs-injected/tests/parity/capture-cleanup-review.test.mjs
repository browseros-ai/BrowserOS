// Delay real browser replies after their effects; never alter the messages themselves.
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
const sleep = (ms) => new Promise((r) => setTimeout(r, ms))
test('capture cleanup after an interrupted reply', {
  skip,
  timeout: 300000,
}, async (t) => {
  const cleanups = [],
    messages = []
  let rule = null
  if (!root) {
    root = mkdtempSync(join(tmpdir(), 'neo-capture-review-'))
    cleanups.push(() => rmSync(root, { recursive: true, force: true }))
  }
  mkdirSync(root, { recursive: true })
  try {
    const bin = buildNeoRun()
    const origin = await serveFixture(cleanups, {
      pages: {
        '/case':
          '<!doctype html><input style="caret-color:red"><div id=box style="height:40px;width:50px">box</div>',
        '/after': '<!doctype html><title>after</title><p>after</p>',
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
          messages.push({ direction: 'send', ...msg })
          if (up.readyState === WebSocket.OPEN) up.send(raw.toString())
          else waiting.push(raw.toString())
        })
        up.on('open', () =>
          waiting.splice(0).forEach((raw) => {
            up.send(raw)
          }),
        )
        up.on('message', (raw) => {
          const msg = JSON.parse(raw.toString()),
            call = calls.get(msg.id)
          messages.push({ direction: 'receive', ...msg })
          const forward = () => {
            if (down.readyState === WebSocket.OPEN) down.send(raw.toString())
          }
          if (rule && !rule.held && call && rule.matches(call, msg)) {
            rule.held = { call, reply: msg }
            rule.holdAt = Date.now()
            if (rule.onHold) rule.onHold()
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
    const cases = [
      {
        name: 'normal',
        method: 'page.screenshot',
        options: { style: 'input{opacity:.5}', omitBackground: true },
        delay: null,
      },
      {
        name: 'timeout-preparing',
        method: 'page.screenshot',
        options: { style: 'input{opacity:.5}' },
        delay: 'prepareForScreenshot',
      },
      {
        name: 'timeout-preparing-locator',
        method: 'page.locator("#box").screenshot',
        options: { style: 'input{opacity:.5}' },
        delay: 'prepareForScreenshot',
      },
      {
        name: 'timeout-background',
        method: 'page.screenshot',
        options: { style: 'input{opacity:.5}', omitBackground: true },
        delay: 'Emulation.setDefaultBackgroundColorOverride',
        background: true,
      },
      {
        name: 'timeout-fonts',
        method: 'page.screenshot',
        options: { style: 'input{opacity:.5}' },
        delay: 'fontsReady',
      },
      {
        name: 'timeout-capturing',
        method: 'page.screenshot',
        options: {
          style: 'input{opacity:.5}',
          mask: true,
          omitBackground: true,
        },
        delay: 'Page.captureScreenshot',
      },
      {
        name: 'navigation-capturing',
        method: 'page.screenshot',
        options: {
          style: 'input{opacity:.5}',
          mask: true,
          omitBackground: true,
        },
        delay: 'Page.captureScreenshot',
        navigate: true,
      },
      {
        name: 'timeout-pdf-stream-reply',
        method: 'page.pdf',
        options: {},
        delay: 'Page.printToPDF',
        runTimeout: true,
      },
      {
        name: 'timeout-pdf-read',
        method: 'page.pdf',
        options: {},
        delay: 'IO.read',
        runTimeout: true,
      },
    ]
    for (const c of cases)
      await t.test(c.name, async () => {
        const { targetId, sessionId } = await openTab(cdp, origin + '/case')
        const start = messages.length
        try {
          const held = new Promise((resolve) => {
            rule = {
              held: null,
              delay: 6500,
              matches: (call) =>
                call.method === c.delay ||
                (call.method === 'Runtime.callFunctionOn' &&
                  call.params.arguments?.[0]?.value === c.delay),
              onHold: resolve,
            }
          })
          const opts = {
            ...c.options,
            ...(!c.runTimeout ? { timeout: c.delay ? 4000 : 10000 } : {}),
          }
          delete opts.mask
          let options = JSON.stringify(opts)
          if (c.options.mask)
            options = options.slice(0, -1) + ',"mask":[page.locator("#box")]}'
          const file = join(root, c.name + '.js')
          writeFileSync(
            file,
            c.background
              ? `let name;try{await ${c.method}(${options})}catch(e){name=e.name}const b=await page.screenshot({clip:{x:0,y:0,width:10,height:10}});return {name,alpha:b[25]===6||b[25]===4}`
              : `try{const b=await ${c.method}(${options});return {bytes:b.length}}catch(e){return {name:e.name}}`,
          )
          const child = spawn(bin, [
            '--cdp-port',
            String(server.address().port),
            '--page',
            targetId,
            '--timeout-ms',
            c.runTimeout ? '4000' : '20000',
            file,
          ])
          let out = '',
            err = ''
          child.stdout.on('data', (d) => (out += d))
          child.stderr.on('data', (d) => (err += d))
          if (c.navigate) {
            await held
            await cdp.send(
              'Page.navigate',
              { url: origin + '/after' },
              sessionId,
            )
          }
          await once(child, 'exit')
          const outcome = JSON.parse(out)
          await sleep(6700)
          const state = (
            await cdp.send(
              'Runtime.evaluate',
              {
                expression: `({styles:document.querySelectorAll('style').length,caret:document.querySelector('input')?.style.caretColor,masks:document.querySelectorAll('x-pw-glass').length,url:location.pathname})`,
                returnByValue: true,
              },
              sessionId,
            )
          ).result.value
          let streamAccessible
          const stream = rule.held?.reply.result?.stream
          if (stream) {
            try {
              const r = await cdp.send('IO.read', { handle: stream }, sessionId)
              streamAccessible = typeof r.data === 'string'
              await cdp.send('IO.close', { handle: stream }, sessionId)
            } catch {
              streamAccessible = false
            }
          }
          const segment = messages.slice(start)
          writeFileSync(
            join(root, c.name + '-wire.json'),
            JSON.stringify(segment, null, 2),
          )
          t.diagnostic(
            JSON.stringify({
              name: c.name,
              status: outcome.status,
              value: outcome.value,
              held: !!rule.held,
              state,
              streamAccessible,
            }),
          )
          if (c.delay)
            assert.ok(rule.held, 'interrupted after the browser effect')
          if (c.background)
            assert.equal(
              outcome.value.alpha,
              false,
              'a later screenshot still has its default opaque background',
            )
          assert.equal(state.styles, 0, 'the screenshot style was restored')
          if (!c.navigate)
            assert.equal(state.caret, 'red', 'the original caret was restored')
          assert.equal(state.masks, 0, 'the mask was removed')
          if (stream)
            assert.equal(
              streamAccessible,
              false,
              'the abandoned PDF stream was closed',
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
