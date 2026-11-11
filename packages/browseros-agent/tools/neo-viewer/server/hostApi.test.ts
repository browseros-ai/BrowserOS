import { afterEach, beforeEach, expect, test } from 'bun:test'
import { mkdtemp, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { configureHostApi, hostFetch, hostWebSocketTarget } from './hostApi'

const TOKEN = 'a'.repeat(40)

/** A stand-in for Neo's host listener: bearer auth on every route, like `host_service`. */
function startFakeHostApi() {
  const seen: Array<{
    path: string
    authorization: string | null
    method: string
    body: string
  }> = []
  const server = Bun.serve({
    hostname: '127.0.0.1',
    port: 0,
    async fetch(req, server) {
      const url = new URL(req.url)
      const authorization = req.headers.get('authorization')
      seen.push({
        path: url.pathname + url.search,
        authorization,
        method: req.method,
        body: await req.text(),
      })
      if (authorization !== `Bearer ${TOKEN}`) {
        return Response.json(
          { code: 'unauthorized', message: 'no' },
          { status: 401 },
        )
      }
      if (url.pathname === '/v1/display')
        return server.upgrade(req)
          ? undefined
          : new Response('no upgrade', { status: 400 })
      if (url.pathname === '/v1/runs/r1/blobs/b0')
        return new Response(new Uint8Array([0, 1, 2, 255]))
      return Response.json({ items: [{ tabId: 7 }] })
    },
    websocket: {
      open(ws) {
        ws.send('RFB 003.008\n')
      },
      message() {},
    },
  })
  return {
    url: new URL(`http://127.0.0.1:${server.port}`),
    seen,
    stop: () => server.stop(true),
  }
}

let dir = ''
let tokenFile = ''
const cleanups: Array<() => unknown> = []

beforeEach(async () => {
  dir = await mkdtemp(join(tmpdir(), 'neo-viewer-host-api-'))
  tokenFile = join(dir, 'host-token')
  await writeFile(tokenFile, `${TOKEN}\n`, { mode: 0o600 })
})

afterEach(async () => {
  await configureHostApi(null)
  while (cleanups.length > 0) await cleanups.pop()?.()
  await rm(dir, { recursive: true, force: true })
})

test('with no host API, hostFetch answers 503 in Neo’s error shape and hostWebSocketTarget throws', async () => {
  const response = await hostFetch('/v1/tabs')
  expect(response.status).toBe(503)
  expect(await response.json()).toMatchObject({ code: 'no_host_api' })
  expect(() => hostWebSocketTarget('/v1/display?mode=view')).toThrow(
    'no host API configured',
  )
})

test('hostFetch sends the bearer token and returns Neo’s answer, JSON or bytes', async () => {
  const neo = startFakeHostApi()
  cleanups.push(neo.stop)
  await configureHostApi({ url: neo.url, tokenFile })
  expect(await (await hostFetch('/v1/tabs?limit=5')).json()).toEqual({
    items: [{ tabId: 7 }],
  })
  expect(
    new Uint8Array(
      await (await hostFetch('/v1/runs/r1/blobs/b0')).arrayBuffer(),
    ),
  ).toEqual(new Uint8Array([0, 1, 2, 255]))
  await hostFetch('/v1/takeovers', {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: '{"tabId":7}',
  })
  expect(
    neo.seen.map(({ path, authorization, method, body }) => [
      method,
      path,
      authorization,
      body,
    ]),
  ).toEqual([
    ['GET', '/v1/tabs?limit=5', `Bearer ${TOKEN}`, ''],
    ['GET', '/v1/runs/r1/blobs/b0', `Bearer ${TOKEN}`, ''],
    ['POST', '/v1/takeovers', `Bearer ${TOKEN}`, '{"tabId":7}'],
  ])
})

test('a host API that does not answer is 502, and only a path on its origin is ever fetched', async () => {
  await configureHostApi({ url: new URL('http://127.0.0.1:1'), tokenFile })
  const response = await hostFetch('/v1/tabs')
  expect(response.status).toBe(502)
  expect(await response.json()).toMatchObject({ code: 'host_api_unreachable' })
  await expect(hostFetch('http://elsewhere.example/v1/tabs')).rejects.toThrow(
    'starts with one "/"',
  )
  await expect(hostFetch('//elsewhere.example/v1/tabs')).rejects.toThrow(
    'starts with one "/"',
  )
})

test('a token file Neo would refuse is refused here too', async () => {
  await writeFile(tokenFile, 'short')
  await expect(
    configureHostApi({ url: new URL('http://127.0.0.1:1'), tokenFile }),
  ).rejects.toThrow('32 or more visible ASCII characters')
})

test('hostWebSocketTarget opens an authenticated upgrade; the token is in a header, never the URL', async () => {
  const neo = startFakeHostApi()
  cleanups.push(neo.stop)
  await configureHostApi({ url: neo.url, tokenFile })
  const target = hostWebSocketTarget('/v1/display?mode=view&followTask=t1')
  expect(target.url).toBe(
    `ws://127.0.0.1:${neo.url.port}/v1/display?mode=view&followTask=t1`,
  )
  expect(target.url).not.toContain(TOKEN)
  expect(target.headers).toEqual({ authorization: `Bearer ${TOKEN}` })
  const socket = new WebSocket(target.url, {
    headers: target.headers,
  } as unknown as string[])
  const greeting = await new Promise<string>((resolve, reject) => {
    socket.onmessage = (event) => resolve(String(event.data))
    socket.onerror = () => reject(new Error('the upgrade failed'))
  })
  socket.close()
  expect(greeting).toBe('RFB 003.008\n')
})
