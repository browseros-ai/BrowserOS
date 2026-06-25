import { afterEach, beforeEach, describe, it } from 'bun:test'
import assert from 'node:assert'
import { CdpBackend } from '@browseros/browser-core/backends/cdp'
import { TIMEOUTS } from '@browseros/shared/constants/timeouts'

class MockWebSocket {
  static instances: MockWebSocket[] = []

  onopen: (() => void) | null = null
  onerror: ((event: unknown) => void) | null = null
  onclose: (() => void) | null = null
  onmessage: ((event: { data: string }) => void) | null = null

  readonly url: string
  sent: string[] = []

  constructor(url: string) {
    this.url = url
    MockWebSocket.instances.push(this)
  }

  send(data: string): void {
    this.sent.push(data)
  }

  close(): void {
    this.onclose?.()
  }

  open(): void {
    this.onopen?.()
  }
}

async function waitFor(
  predicate: () => boolean,
  timeoutMs = 1_000,
): Promise<void> {
  const start = Date.now()
  while (Date.now() - start < timeoutMs) {
    if (predicate()) return
    await Bun.sleep(1)
  }
  throw new Error('Timed out waiting for condition')
}

describe('CdpBackend', () => {
  const originalFetch = globalThis.fetch
  const originalWebSocket = globalThis.WebSocket
  const originalConnectTimeout = TIMEOUTS.CDP_CONNECT
  const originalReconnectDelay = TIMEOUTS.CDP_RECONNECT_DELAY
  let fetchUrls: string[] = []
  let failIpv4Discovery = false
  let failAllDiscovery = false
  let wsHost = '127.0.0.1'
  let originalExit: typeof process.exit

  beforeEach(() => {
    MockWebSocket.instances = []
    fetchUrls = []
    failIpv4Discovery = false
    failAllDiscovery = false
    wsHost = '127.0.0.1'
    originalExit = process.exit

    ;(TIMEOUTS as unknown as { CDP_CONNECT: number }).CDP_CONNECT = 200
    ;(
      TIMEOUTS as unknown as { CDP_RECONNECT_DELAY: number }
    ).CDP_RECONNECT_DELAY = 1

    globalThis.fetch = (async (input: string | URL | Request) => {
      const url = String(input)
      fetchUrls.push(url)
      if (failAllDiscovery) {
        throw new Error('Unable to connect')
      }
      if (failIpv4Discovery && url.includes('127.0.0.1')) {
        throw new Error('Unable to connect')
      }
      const id = fetchUrls.length
      return {
        ok: true,
        status: 200,
        statusText: 'OK',
        json: async () => ({
          webSocketDebuggerUrl: `ws://${wsHost}:9222/devtools/browser/${id}`,
        }),
      } as Response
    }) as typeof fetch

    globalThis.WebSocket = MockWebSocket as unknown as typeof WebSocket
  })

  afterEach(() => {
    globalThis.fetch = originalFetch
    globalThis.WebSocket = originalWebSocket
    process.exit = originalExit
    ;(TIMEOUTS as unknown as { CDP_CONNECT: number }).CDP_CONNECT =
      originalConnectTimeout
    ;(
      TIMEOUTS as unknown as { CDP_RECONNECT_DELAY: number }
    ).CDP_RECONNECT_DELAY = originalReconnectDelay
  })

  it('falls back from 127.0.0.1 to localhost for /json/version', async () => {
    failIpv4Discovery = true
    wsHost = 'localhost'
    const cdp = new CdpBackend({ port: 9222 })
    const connectPromise = cdp.connect()

    await waitFor(() => MockWebSocket.instances.length === 1)
    MockWebSocket.instances[0]?.open()
    await connectPromise

    assert.strictEqual(fetchUrls[0], 'http://127.0.0.1:9222/json/version')
    assert.strictEqual(fetchUrls[1], 'http://localhost:9222/json/version')
    assert.strictEqual(
      MockWebSocket.instances[0]?.url,
      'ws://localhost:9222/devtools/browser/2',
    )
    await cdp.disconnect()
  })

  it('prefers the last successful discovery host during reconnect', async () => {
    failIpv4Discovery = true
    wsHost = 'localhost'
    const cdp = new CdpBackend({ port: 9222 })
    const connectPromise = cdp.connect()

    await waitFor(() => MockWebSocket.instances.length === 1)
    const ws1 = MockWebSocket.instances[0]
    ws1?.open()
    await connectPromise

    assert.strictEqual(fetchUrls[0], 'http://127.0.0.1:9222/json/version')
    assert.strictEqual(fetchUrls[1], 'http://localhost:9222/json/version')

    ws1?.close()

    await waitFor(() => fetchUrls.length >= 3)
    assert.strictEqual(fetchUrls[2], 'http://localhost:9222/json/version')
    await cdp.disconnect()
  })

  it('reconnects again when a socket closes during reconnect', async () => {
    const cdp = new CdpBackend({ port: 9222 })
    const connectPromise = cdp.connect()

    await waitFor(() => MockWebSocket.instances.length === 1)
    const ws1 = MockWebSocket.instances[0]
    ws1?.open()
    await connectPromise
    assert.strictEqual(cdp.isConnected(), true)

    ws1?.close()

    await waitFor(() => MockWebSocket.instances.length >= 2)
    const ws2 = MockWebSocket.instances[1]
    ws2?.open()
    ws2?.close()

    await waitFor(() => MockWebSocket.instances.length >= 3)
    const ws3 = MockWebSocket.instances[2]
    ws3?.open()

    await waitFor(() => cdp.isConnected())
    assert.strictEqual(cdp.isConnected(), true)
    assert(fetchUrls.length >= 3)
    await cdp.disconnect()
  })

  it('can disable process exit when reconnect retries are exhausted', async () => {
    let exitCalled = false
    process.exit = (() => {
      exitCalled = true
      throw new Error('process.exit should not be called')
    }) as unknown as typeof process.exit

    const cdp = new CdpBackend({ port: 9222, exitOnReconnectFailure: false })
    const connectPromise = cdp.connect()

    await waitFor(() => MockWebSocket.instances.length === 1)
    const ws1 = MockWebSocket.instances[0]
    ws1?.open()
    await connectPromise
    assert.strictEqual(cdp.isConnected(), true)

    failAllDiscovery = true
    ws1?.close()

    await waitFor(() => fetchUrls.length >= 10)
    await Bun.sleep(5)

    assert.strictEqual(exitCalled, false)
    assert.strictEqual(cdp.isConnected(), false)
    await cdp.disconnect()
  })
})
