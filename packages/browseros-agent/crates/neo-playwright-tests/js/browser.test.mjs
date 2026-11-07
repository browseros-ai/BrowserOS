/** Offline transport checks and an opt-in owned-browser lifecycle gate. */
import assert from 'node:assert/strict'
import { createHash } from 'node:crypto'
import { mkdir, mkdtemp, readFile, writeFile } from 'node:fs/promises'
import { createServer } from 'node:http'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { test } from 'node:test'
import { Cdp, connect, OwnedBrowser } from './browser.mjs'

/** Implements just enough RFC 6455 for the real built-in WebSocket client, without packages. */
async function fakeCdp(receive) {
  const sockets = new Set()
  // Bun's node:http upgrade socket is a compatibility wrapper. Its native server keeps
  // these offline checks about the shared WebSocket client rather than that wrapper.
  if (globalThis.Bun) {
    const server = Bun.serve({
      hostname: '127.0.0.1',
      port: 0,
      fetch(request, owner) {
        if (!owner.upgrade(request))
          return new Response('upgrade required', { status: 426 })
      },
      websocket: {
        open(socket) {
          sockets.add(socket)
        },
        close(socket) {
          sockets.delete(socket)
        },
        message(socket, data) {
          receive(JSON.parse(String(data)), {
            send: (value) => socket.send(JSON.stringify(value)),
            raw: (value) => socket.send(value),
            close: () => socket.terminate(),
          })
        },
      },
    })
    return {
      endpoint: `ws://127.0.0.1:${server.port}/devtools/browser/offline`,
      close: async () => {
        for (const socket of sockets) socket.terminate()
        server.stop(true)
      },
    }
  }
  const server = createServer()
  const ready = new Promise((resolveReady, reject) => {
    server.once('listening', resolveReady)
    server.once('error', reject)
  })
  server.on('upgrade', (request, socket, head) => {
    sockets.add(socket)
    socket.on('error', () => {})
    socket.once('close', () => sockets.delete(socket))
    const accept = createHash('sha1')
      .update(
        `${request.headers['sec-websocket-key']}258EAFA5-E914-47DA-95CA-C5AB0DC85B11`,
      )
      .digest('base64')
    socket.write(
      `HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: ${accept}\r\n\r\n`,
    )
    function frame(text, opcode = 1) {
      const payload = Buffer.from(text)
      let prefix
      if (payload.length < 126)
        prefix = Buffer.from([0x80 | opcode, payload.length])
      else if (payload.length < 65536) {
        prefix = Buffer.alloc(4)
        prefix[0] = 0x80 | opcode
        prefix[1] = 126
        prefix.writeUInt16BE(payload.length, 2)
      } else {
        prefix = Buffer.alloc(10)
        prefix[0] = 0x80 | opcode
        prefix[1] = 127
        prefix.writeBigUInt64BE(BigInt(payload.length), 2)
      }
      return Buffer.concat([prefix, payload])
    }
    const peer = {
      send: (value) => socket.write(frame(JSON.stringify(value))),
      raw: (text) => socket.write(frame(text)),
      close: () => socket.destroy(),
    }
    let buffered = Buffer.alloc(0)
    let fragments = []
    function data(chunk) {
      buffered = Buffer.concat([buffered, chunk])
      while (buffered.length >= 2) {
        const first = buffered[0]
        const opcode = first & 0x0f
        let length = buffered[1] & 0x7f
        let offset = 2
        if (length === 126) {
          if (buffered.length < 4) return
          length = buffered.readUInt16BE(2)
          offset = 4
        } else if (length === 127) {
          if (buffered.length < 10) return
          length = Number(buffered.readBigUInt64BE(2))
          offset = 10
        }
        const masked = Boolean(buffered[1] & 0x80)
        if (buffered.length < offset + (masked ? 4 : 0) + length) return
        const mask = masked ? buffered.subarray(offset, offset + 4) : null
        offset += masked ? 4 : 0
        const payload = Buffer.from(buffered.subarray(offset, offset + length))
        buffered = buffered.subarray(offset + length)
        if (mask)
          for (let index = 0; index < payload.length; index++)
            payload[index] ^= mask[index % 4]
        if (opcode === 8) {
          socket.end(Buffer.from([0x88, 0]))
          return
        }
        if (opcode === 9) {
          socket.write(frame(payload, 10))
          continue
        }
        if (opcode !== 0 && opcode !== 1)
          throw new Error('Unexpected fake-server frame opcode')
        fragments.push(payload)
        if (first & 0x80) {
          const message = JSON.parse(Buffer.concat(fragments).toString())
          fragments = []
          receive(message, peer)
        }
      }
    }
    socket.on('data', data)
    if (head.length) data(head)
  })
  server.listen(0, '127.0.0.1')
  await ready
  return {
    endpoint: `ws://127.0.0.1:${server.address().port}/devtools/browser/offline`,
    close: async () => {
      for (const socket of sockets) socket.destroy()
      await new Promise((resolveClose) => server.close(resolveClose))
    },
  }
}

test('CDP correlates concurrent replies, retains session events, and reports protocol errors', async (t) => {
  const requests = []
  const server = await fakeCdp((message, peer) => {
    requests.push(message)
    if (requests.length === 2) {
      peer.send({
        method: 'Runtime.bindingCalled',
        params: { payload: 'observed' },
        sessionId: 'session-b',
      })
      peer.send({
        id: requests[1].id,
        result: { second: true },
        sessionId: requests[1].sessionId,
      })
      peer.send({
        id: requests[0].id,
        result: { first: true },
        sessionId: requests[0].sessionId,
      })
    } else if (message.method === 'Missing.method') {
      peer.send({
        id: message.id,
        error: { code: -32601, message: 'missing method' },
      })
    }
  })
  t.after(() => server.close())
  const cdp = await connect(server.endpoint)
  t.after(() => cdp.close())
  const events = []
  const unsubscribe = cdp.subscribe((event) => events.push(event))
  const results = await Promise.all([
    cdp.send('First.method', { first: 1 }, 'session-a'),
    cdp.send('Second.method', { second: 2 }, 'session-b'),
  ])
  assert.deepEqual(results, [{ first: true }, { second: true }])
  assert.deepEqual(
    requests
      .slice(0, 2)
      .map(({ params, sessionId }) => ({ params, sessionId })),
    [
      { params: { first: 1 }, sessionId: 'session-a' },
      { params: { second: 2 }, sessionId: 'session-b' },
    ],
  )
  assert.deepEqual(events, [
    {
      method: 'Runtime.bindingCalled',
      params: { payload: 'observed' },
      sessionId: 'session-b',
    },
  ])
  unsubscribe()
  await assert.rejects(
    cdp.send('Missing.method'),
    (error) => error.code === -32601 && /missing method/.test(error.message),
  )
})

test('a disconnect rejects every pending request and later sends', async (t) => {
  let requestCount = 0
  const server = await fakeCdp((_, peer) => {
    if (++requestCount === 2) peer.close()
  })
  t.after(() => server.close())
  const cdp = await Cdp.connect(server.endpoint)
  t.after(() => cdp.close())
  const disconnected = new Promise((resolveDisconnected) =>
    cdp.onDisconnect(resolveDisconnected),
  )
  const pending = [cdp.send('First.pending'), cdp.send('Second.pending')]
  const results = await Promise.allSettled(pending)
  assert.equal(results.length, 2)
  assert.ok(
    results.every(
      (result) =>
        result.status === 'rejected' && /CDP/.test(result.reason.message),
    ),
  )
  assert.match((await disconnected).message, /CDP/)
  await assert.rejects(cdp.send('After.disconnect'), /CDP/)
})

test('reply deadlines leave the connection usable and local close rejects pending requests', async (t) => {
  const server = await fakeCdp((message, peer) => {
    if (message.method === 'Respond.method')
      peer.send({ id: message.id, result: { alive: true } })
  })
  t.after(() => server.close())
  const cdp = await Cdp.connect(server.endpoint, { timeoutMs: 40 })
  t.after(() => cdp.close())
  await assert.rejects(cdp.send('Never.replies'), /reply timed out/)
  assert.deepEqual(await cdp.send('Respond.method'), { alive: true })
  const pending = assert.rejects(
    cdp.send('Pending.atClose'),
    /connection closed/,
  )
  await cdp.close()
  await pending
  await cdp.close()
})

test('malformed protocol traffic rejects pending requests instead of losing evidence', async (t) => {
  const server = await fakeCdp((_, peer) => peer.raw('not json'))
  t.after(() => server.close())
  const cdp = await Cdp.connect(server.endpoint)
  t.after(() => cdp.close())
  await assert.rejects(cdp.send('Pending.onMalformed'), SyntaxError)
  await assert.rejects(cdp.send('After.malformed'), SyntaxError)
})

test('target detachment cancels only that session and preserves the original event envelope', async (t) => {
  let peer
  const requests = []
  const server = await fakeCdp((message, client) => {
    peer = client
    requests.push(message)
    if (requests.length === 2) {
      peer.send({
        method: 'Target.detachedFromTarget',
        params: { sessionId: 'gone' },
      })
      peer.send({ id: requests[1].id, result: { alive: true } })
    }
  })
  t.after(() => server.close())
  const cdp = await connect(server.endpoint)
  t.after(() => cdp.close())
  const events = []
  cdp.subscribe((event) => events.push(event))
  const gone = assert.rejects(
    cdp.send('Pending.resume', {}, 'gone'),
    (error) => error.code === 'CDP_SESSION_CLOSED',
  )
  assert.deepEqual(await cdp.send('Other.target', {}, 'alive'), { alive: true })
  await gone
  assert.deepEqual(events, [
    { method: 'Target.detachedFromTarget', params: { sessionId: 'gone' } },
  ])
})

// Opt in with an explicit executable. Ordinary Node/Bun checks stay offline and never
// acquire a user's browser. The live test retains raw evidence and process cleanup proof.
const liveTest = process.env.NEO_OBSERVER_BROWSER ? test : test.skip
liveTest(
  'owned browser records empty-document navigation, closure, and explicit observation gaps',
  {
    timeout: 90_000,
  },
  async () => {
    const root = process.env.NEO_OBSERVER_ARTIFACT_DIR ?? tmpdir()
    await mkdir(root, { recursive: true })
    const outputDir = await mkdtemp(join(root, 'observer-lifecycle-'))
    const server = createServer((_, response) =>
      response.end('<html><head></head><body></body></html>'),
    )
    await new Promise((ready) => server.listen(0, '127.0.0.1', ready))
    let browser
    let actor
    try {
      browser = await OwnedBrowser.start({
        executable: process.env.NEO_OBSERVER_BROWSER,
        outputDir,
      })
      actor = await connect(browser.endpoint)
      const { sessionId } = await actor.send('Target.attachToTarget', {
        targetId: browser.pageId,
        flatten: true,
      })
      await actor.send('Page.enable', {}, sessionId)
      const url = `http://127.0.0.1:${server.address().port}/`
      const loaded = new Promise((ready, reject) => {
        const timeout = setTimeout(() => {
          unsubscribe()
          reject(new Error('Navigation did not load'))
        }, 10_000)
        const unsubscribe = actor.subscribe((event) => {
          if (
            event.sessionId === sessionId &&
            event.method === 'Page.loadEventFired'
          ) {
            clearTimeout(timeout)
            unsubscribe()
            ready()
          }
        })
      })
      await actor.send('Page.navigate', { url }, sessionId)
      await loaded
      assert.equal(
        (
          await actor.send(
            'Runtime.evaluate',
            { expression: 'location.href', returnByValue: true },
            sessionId,
          )
        ).result.value,
        url,
      )
      const navigation = await browser.snapshot()
      await writeFile(
        join(outputDir, 'navigation.json'),
        JSON.stringify(navigation, null, 2),
      )
      assert.equal(
        navigation.status,
        'complete',
        JSON.stringify(navigation.incomplete),
      )
      assert.equal(navigation.pages[0].documents[0].retired, true)
      assert.ok(navigation.pages[0].documents[0].dom)

      const { targetId } = await actor.send('Target.createTarget', {
        url: 'about:blank',
      })
      const { sessionId: nextSession } = await actor.send(
        'Target.attachToTarget',
        { targetId, flatten: true },
      )
      await actor.send('Runtime.enable', {}, nextSession)
      await actor.send(
        'Runtime.evaluate',
        { expression: 'location.href', returnByValue: true },
        nextSession,
      )
      assert.equal((await browser.snapshot()).status, 'complete')
      await actor.send('Target.closeTarget', { targetId })
      const closed = await browser.snapshot()
      await writeFile(
        join(outputDir, 'closed.json'),
        JSON.stringify(closed, null, 2),
      )
      assert.equal(closed.status, 'complete', JSON.stringify(closed.incomplete))

      await actor.send(
        'Runtime.evaluate',
        {
          expression: `const host = document.body.appendChild(document.createElement('div'));
        const root = host.attachShadow({mode:'closed'});
        const input = root.appendChild(document.createElement('input'));
        input.dispatchEvent(new InputEvent('input', {bubbles:true, composed:false, data:'hidden'}));`,
        },
        sessionId,
      )
      const shadow = await browser.snapshot()
      await writeFile(
        join(outputDir, 'shadow.json'),
        JSON.stringify(shadow, null, 2),
      )
      assert.ok(
        shadow.incomplete.some(
          (gap) => gap.reason === 'closed-shadow-root-input-not-observed',
        ),
      )
      assert.ok(!shadow.events.some((event) => event.data === 'hidden'))

      await actor.send('Page.crash', {}, sessionId).catch(() => {})
      await actor.send('Target.closeTarget', { targetId: browser.pageId })
      const lost = await browser.snapshot()
      await writeFile(
        join(outputDir, 'lost.json'),
        JSON.stringify(lost, null, 2),
      )
      assert.ok(
        lost.incomplete.some(
          (gap) => gap.reason === 'closed-document-without-terminal',
        ),
      )
    } finally {
      await actor?.close()
      await browser?.close()
      await new Promise((closed) => server.close(closed))
    }
    const cleanup = JSON.parse(
      await readFile(join(outputDir, 'cleanup.json'), 'utf8'),
    )
    assert.equal(cleanup.status, 'complete')
    assert.deepEqual(cleanup.survivors, [])
    console.log(`Observer lifecycle evidence saved in ${outputDir}`)
  },
)
