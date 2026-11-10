/** Neo: Exercises the real Node SDK's synchronous door, reply ordering, idle wake and shutdown. */
import assert from 'node:assert/strict'
import fs from 'node:fs/promises'
import http from 'node:http'
import path from 'node:path'
import test from 'node:test'
import { setTimeout as sleep } from 'node:timers/promises'
import { agent } from '../js/prepare.mjs'
import { launchBrowser } from './browser.mjs'
import { openSdk } from './loader.mjs'

test('real SDK in Node keeps getters synchronous, delivers events and finishes pending wakes', {
  timeout: 60_000,
}, async () => {
  assert.ok(process.env.NEO_E2E_BROWSER, 'Set NEO_E2E_BROWSER')
  const root =
    process.env.NEO_NODE_OUTPUT_DIR ||
    path.resolve(agent, '../../.llm/neo-node-results')
  await fs.mkdir(root, { recursive: true })
  const output = await fs.mkdtemp(path.join(root, 'smoke-'))
  const server = http.createServer((_, response) => {
    response.writeHead(200, { 'content-type': 'text/html' })
    response.end(
      '<!doctype html><title>Node smoke</title><script>setTimeout(() => console.log("idle-event"), 500)</script>',
    )
  })
  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve))
  let owned
  let sdk
  try {
    owned = await launchBrowser(process.env.NEO_E2E_BROWSER, output)
    let receivedUncaught
    const uncaught = new Promise((resolve) => {
      receivedUncaught = resolve
    })
    sdk = await openSdk(
      owned.port,
      { timeoutMs: 45_000, artifactsDir: path.join(output, 'artifacts') },
      { onUncaught: receivedUncaught },
    )
    const browser = sdk.context.browser()
    const context = await browser.newContext({
      viewport: { width: 1280, height: 720 },
    })
    const page = await context.newPage()
    assert.equal(page.context(), context)
    assert.equal(context.browser(), browser)
    assert.equal(page.url(), 'about:blank')
    assert.equal(typeof page.url(), 'string')
    assert.equal(page.url() instanceof Promise, false)
    const order = []
    const consoleEvents = []
    page.on('request', (request) => {
      order.push(`request:${request.url()}`)
    })
    page.on('response', (response) => {
      order.push(`response:${response.status()}`)
    })
    page.on('console', (message) => {
      consoleEvents.push(message.text())
    })
    const url = `http://127.0.0.1:${server.address().port}/`
    const response = await page.goto(url)
    order.push('continued')
    assert.equal(response.status(), 200)
    assert.equal(page.url(), url)
    const requestIndex = order.findIndex((value) =>
      value.startsWith('request:'),
    )
    assert.ok(
      requestIndex >= 0 && requestIndex < order.indexOf('continued'),
      order.join(', '),
    )
    assert.ok(
      order.indexOf('response:200') >= 0 &&
        order.indexOf('response:200') < order.indexOf('continued'),
      order.join(', '),
    )
    assert.deepEqual(consoleEvents, [])
    // No browser call while the timer runs: delivery must come from nextEvents, not a reply.
    await sleep(900)
    assert.deepEqual(consoleEvents, ['idle-event'])
    page.on('console', () => {
      throw new Error('intentional listener error')
    })
    await page.evaluate(() => console.log('throw-now'))
    const error = await Promise.race([
      uncaught,
      sleep(3000, undefined, { ref: false }).then(() => {
        throw new Error('Uncaught listener did not reach host')
      }),
    ])
    assert.match(error.message, /intentional listener error/)
    const outcome = JSON.parse(await sdk.finish())
    sdk = undefined
    await fs.writeFile(
      path.join(output, 'outcome.json'),
      JSON.stringify(outcome, null, 2),
    )
    await fs.writeFile(
      path.join(output, 'proof.json'),
      JSON.stringify(
        {
          order,
          consoleEvents,
          uncaught: error.message,
          synchronousUrl: true,
          finished: true,
        },
        null,
        2,
      ),
    )
    console.log(`Smoke evidence: ${output}`)
  } finally {
    try {
      await sdk?.finish()
    } finally {
      await owned?.close()
      await new Promise((resolve) => server.close(resolve))
    }
  }
})
