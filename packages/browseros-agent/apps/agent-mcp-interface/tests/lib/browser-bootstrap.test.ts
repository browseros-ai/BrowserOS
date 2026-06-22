/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Bootstrap covers the connect / disconnect contract the cockpit
 * relies on when running standalone. The real `CdpBackend` is
 * injected via `BrowserBootstrapDeps`, so this test never opens a
 * socket.
 */

import { describe, expect, test } from 'bun:test'
import type { BrowserSession } from '@browseros/server/browser/core/session'
import {
  bootstrapBrowserosBrowser,
  type CdpClient,
} from '../../src/lib/browser-bootstrap'

interface StubCdp extends CdpClient {
  readonly connectCalls: number
  readonly disconnectCalls: number
}

function makeStubCdp(opts: { connectThrows?: Error } = {}): StubCdp {
  let connectCalls = 0
  let disconnectCalls = 0
  return {
    get connectCalls() {
      return connectCalls
    },
    get disconnectCalls() {
      return disconnectCalls
    },
    connect: async () => {
      connectCalls++
      if (opts.connectThrows) throw opts.connectThrows
    },
    disconnect: async () => {
      disconnectCalls++
    },
  }
}

const fakeSession = { tag: 'fake-session' } as unknown as BrowserSession

describe('bootstrapBrowserosBrowser', () => {
  test('connects, builds a session, and returns a disconnect() that drops the cdp', async () => {
    const cdp = makeStubCdp()
    const seenCdps: CdpClient[] = []
    const result = await bootstrapBrowserosBrowser({
      inject: {
        cdpFactory: () => cdp,
        buildSession: (c) => {
          seenCdps.push(c)
          return fakeSession
        },
      },
    })
    expect(result).not.toBeNull()
    expect(result?.session).toBe(fakeSession)
    expect(cdp.connectCalls).toBe(1)
    expect(seenCdps).toEqual([cdp])
    await result?.disconnect()
    expect(cdp.disconnectCalls).toBe(1)
  })

  test('returns null when the cdp connect throws and never builds a session', async () => {
    const cdp = makeStubCdp({ connectThrows: new Error('ECONNREFUSED') })
    let buildSessionCalls = 0
    const result = await bootstrapBrowserosBrowser({
      inject: {
        cdpFactory: () => cdp,
        buildSession: () => {
          buildSessionCalls++
          return fakeSession
        },
      },
    })
    expect(result).toBeNull()
    expect(cdp.connectCalls).toBe(1)
    expect(buildSessionCalls).toBe(0)
  })

  test('disconnect() swallows underlying errors so callers can call it from a signal handler', async () => {
    const cdp: CdpClient = {
      connect: async () => undefined,
      disconnect: async () => {
        throw new Error('socket already gone')
      },
    }
    const result = await bootstrapBrowserosBrowser({
      inject: {
        cdpFactory: () => cdp,
        buildSession: () => fakeSession,
      },
    })
    expect(result).not.toBeNull()
    // The disconnect must not throw — the signal-handler path in
    // main.ts relies on this so it can always reach process.exit.
    await expect(result?.disconnect()).resolves.toBeUndefined()
  })

  test('cdpFactory is invoked with env.cdpPort so a misconfigured port is observable', async () => {
    const seenPorts: number[] = []
    const cdp = makeStubCdp()
    await bootstrapBrowserosBrowser({
      inject: {
        cdpFactory: (port) => {
          seenPorts.push(port)
          return cdp
        },
        buildSession: () => fakeSession,
      },
    })
    expect(seenPorts).toHaveLength(1)
    expect(seenPorts[0]).toBeGreaterThan(0)
    expect(seenPorts[0]).toBeLessThanOrEqual(65535)
  })
})
