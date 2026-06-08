/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Smoke tests for the generated @browseros/cdp-protocol factory. They verify
 * the protocol proxy exposes the expected domains and forwards method calls and
 * event subscriptions to the underlying transport with the right CDP names.
 */

import { describe, it } from 'bun:test'
import assert from 'node:assert'

import { createProtocolApi } from '@browseros/cdp-protocol/create-api'

function makeApi() {
  const sent: { method: string; params?: Record<string, unknown> }[] = []
  const subscribed: string[] = []
  const send = async (method: string, params?: Record<string, unknown>) => {
    sent.push({ method, params })
    return { ok: true }
  }
  const on = (event: string) => {
    subscribed.push(event)
    return () => {}
  }
  return { api: createProtocolApi(send, on), sent, subscribed }
}

describe('@browseros/cdp-protocol create-api', () => {
  it('exposes the core CDP domains', () => {
    const { api } = makeApi()
    for (const domain of [
      'Browser',
      'Page',
      'DOM',
      'Input',
      'Network',
      'Target',
      'Runtime',
    ] as const) {
      assert.ok(api[domain], `missing CDP domain: ${domain}`)
    }
  })

  it('forwards a method call as "Domain.method"', async () => {
    const { api, sent } = makeApi()
    await api.Page.navigate({ url: 'https://example.com' })
    assert.deepStrictEqual(sent, [
      { method: 'Page.navigate', params: { url: 'https://example.com' } },
    ])
  })

  it('forwards event subscriptions as "Domain.event"', () => {
    const { api, subscribed } = makeApi()
    api.Target.on('targetCreated', () => {})
    assert.deepStrictEqual(subscribed, ['Target.targetCreated'])
  })

  it('returns a callable unsubscribe handle from .on()', () => {
    const { api } = makeApi()
    const unsubscribe = api.Target.on('targetCreated', () => {})
    assert.strictEqual(typeof unsubscribe, 'function')
    // Must not throw when invoked.
    unsubscribe()
  })

  // Regression guard for the generated protocol: a domain accidentally dropped
  // during regeneration is a silent, far-reaching break. Update this list
  // deliberately when the CDP surface changes.
  it('exposes exactly the expected set of domains', () => {
    const expected = [
      'Accessibility',
      'Animation',
      'Audits',
      'Autofill',
      'Bookmarks',
      'BackgroundService',
      'BluetoothEmulation',
      'Browser',
      'CSS',
      'CacheStorage',
      'Cast',
      'DOM',
      'DOMDebugger',
      'DOMSnapshot',
      'DOMStorage',
      'DeviceAccess',
      'DeviceOrientation',
      'Emulation',
      'EventBreakpoints',
      'Extensions',
      'FedCm',
      'Fetch',
      'FileSystem',
      'HeadlessExperimental',
      'History',
      'IO',
      'IndexedDB',
      'Input',
      'Inspector',
      'LayerTree',
      'Log',
      'Media',
      'Memory',
      'Network',
      'Overlay',
      'PWA',
      'Page',
      'Performance',
      'PerformanceTimeline',
      'Preload',
      'Security',
      'ServiceWorker',
      'SmartCardEmulation',
      'Storage',
      'SystemInfo',
      'Target',
      'Tethering',
      'Tracing',
      'WebAudio',
      'WebAuthn',
      'Console',
      'Debugger',
      'HeapProfiler',
      'Profiler',
      'Runtime',
      'Schema',
    ]
    const { api } = makeApi()
    assert.deepStrictEqual(
      Object.keys(api as Record<string, unknown>).sort(),
      [...expected].sort(),
    )
  })
})
