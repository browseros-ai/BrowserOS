/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Tests for browser/extension-bridge.ts — L3 Extension Message Bridge
 *
 * Two-phase service worker discovery:
 *   1. ServiceWorker.enable() -> startWorker(scopeURL) -> getTargets()
 *   2. Attach to SW -> Runtime.evaluate(chrome.runtime.sendMessage)
 *
 * TDD order (worst-first):
 *   Zone 4: error propagation (no SW, timeout, attach failure)
 *   Zone 5: state mutation (SW suspends between discover and attach)
 *   Zone 1: empty (no extensions, no running SWs)
 *   Zone 3: multi-flag (extension with both SW and background_page)
 *   Happy path
 */

import { afterEach, beforeEach, describe, expect, mock, test } from 'bun:test'
import type { ProtocolApi } from '@browseros/cdp-protocol/protocol-api'
import type { CdpBackend, CdpTarget } from './backends/types'
import {
  listMessageableExtensions,
  sendExtensionMessage,
  deriveScopeURL,
} from './extension-bridge'

// ── Mock factories ──

interface MockCdpBackendOverrides {
  getTargets?: CdpTarget[]
  attachToTarget?: { sessionId: string }
  evaluateResult?: { result: { type: string; value?: unknown } }
  evaluateError?: string
  swStartWorkerError?: string
}

function createMockBackend(overrides: MockCdpBackendOverrides = {}): CdpBackend {
  const sessionApi = {
    Runtime: {
      enable: mock(async () => {}),
      evaluate: mock(async () => {
        if (overrides.evaluateError) {
          throw new Error(overrides.evaluateError)
        }
        return overrides.evaluateResult ?? { result: { type: 'undefined' } }
      }),
      on: mock(() => {}),
    },
  } as unknown as ProtocolApi

  return {
    connect: mock(async () => {}),
    disconnect: mock(async () => {}),
    isConnected: mock(() => true),
    getTargets: mock(async () => overrides.getTargets ?? []),
    session: mock((() => sessionApi) as any),
    onSessionEvent: mock(() => () => {}),
    ServiceWorker: {
      enable: mock(async () => {}),
      startWorker: mock(async () => {
        if (overrides.swStartWorkerError) {
          throw new Error(overrides.swStartWorkerError)
        }
      }),
    },
    Target: {
      setAutoAttach: mock(async () => {}),
      on: mock(() => {}),
      getTargets: mock(async () => ({
        targetInfos: (overrides.getTargets ?? []).map((t) => ({
          targetId: t.id,
          type: t.type,
          title: t.title,
          url: t.url,
        })),
      })),
      attachToTarget: mock(async () =>
        overrides.attachToTarget ?? { sessionId: 'sw-sess-1' },
      ),
      detachFromTarget: mock(async () => {}),
      createTarget: mock(async () => ({ targetId: 't-1' })),
      closeTarget: mock(async () => ({ success: true })),
      activateTarget: mock(async () => {}),
    },
    Extensions: {
      loadUnpacked: mock(async () => ({ id: 'ext-1' })),
      uninstall: mock(async () => {}),
      getStorageItems: mock(async () => ({ data: {} })),
      setStorageItems: mock(async () => {}),
      removeStorageItems: mock(async () => {}),
      clearStorageItems: mock(async () => {}),
    },
    Page: {
      enable: mock(async () => {}),
      disable: mock(async () => {}),
      on: mock(() => {}),
      getFrameTree: mock(async () => ({
        frameTree: { frame: { id: 'f1', url: 'about:blank' } },
      })),
    },
    Runtime: {
      enable: mock(async () => {}),
      evaluate: mock(async () => ({ result: { type: 'undefined' } })),
      on: mock(() => {}),
    },
    Emulation: {
      setDeviceMetricsOverride: mock(async () => {}),
    },
    Browser: {
      getWindowForTarget: mock(async () => ({ windowId: 1, bounds: {} })),
      setWindowBounds: mock(async () => {}),
    },
  } as unknown as CdpBackend
}

// ── Zone 4: Error propagation ──

describe('extension-bridge — Zone 4: error propagation', () => {
  test('sendExtensionMessage throws when no service worker target found', async () => {
    const backend = createMockBackend({ getTargets: [] })

    await expect(
      sendExtensionMessage(backend, 'ext-no-sw', { action: 'test' }),
    ).rejects.toThrow('No discoverable target')
  })

  test('sendExtensionMessage throws when attachToTarget fails', async () => {
    const backend = createMockBackend({
      getTargets: [
        {
          id: 'sw-1',
          type: 'service_worker',
          title: 'Extension BG',
          url: 'chrome-extension://ext-1/background.js',
        },
      ],
      attachToTarget: undefined as any,
    })
    // Make attach fail
    backend.Target.attachToTarget = mock(async () => {
      throw new Error('Attach failed')
    })

    await expect(
      sendExtensionMessage(backend, 'ext-1', { action: 'test' }),
    ).rejects.toThrow('Attach failed')
  })

  test('sendExtensionMessage throws on Runtime.evaluate error', async () => {
    const backend = createMockBackend({
      getTargets: [
        {
          id: 'sw-1',
          type: 'service_worker',
          title: 'Ext SW',
          url: 'chrome-extension://ext-1/background.js',
        },
      ],
      evaluateError: 'Extension context invalidated',
    })

    await expect(
      sendExtensionMessage(backend, 'ext-1', { action: 'test' }),
    ).rejects.toThrow('Extension context invalidated')
  })
})

// ── Zone 5: State mutation ──

describe('extension-bridge — Zone 5: state mutation', () => {
  test('sendExtensionMessage retries on SW suspend between discover and attach', async () => {
    let attachCalls = 0
    const backend = createMockBackend({
      getTargets: [
        {
          id: 'sw-1',
          type: 'service_worker',
          title: 'Ext SW',
          url: 'chrome-extension://ext-1/sw.js',
        },
      ],
    })

    // First attach fails (SW suspended), second succeeds
    backend.Target.attachToTarget = mock(async () => {
      attachCalls++
      if (attachCalls === 1) throw new Error('Target closed')
      return { sessionId: 'sw-sess-2' }
    })

    // Should not throw — retry kicks in
    // Note: this test validates retry exists, not that it succeeds
    await expect(
      sendExtensionMessage(backend, 'ext-1', { action: 'ping' }),
    ).resolves.toBeDefined()
    expect(attachCalls).toBeGreaterThanOrEqual(2)
  })
})

// ── Zone 1: Empty results ──

describe('extension-bridge — Zone 1: empty results', () => {
  test('listMessageableExtensions returns empty array when no SW targets', async () => {
    const backend = createMockBackend({ getTargets: [] })

    const result = await listMessageableExtensions(backend)

    expect(result).toEqual([])
  })

  test('listMessageableExtensions filters out non-service_worker targets', async () => {
    const backend = createMockBackend({
      getTargets: [
        { id: 'page-1', type: 'page', title: 'Tab', url: 'https://example.com' },
        { id: 'bg-1', type: 'background_page', title: 'Old ext', url: 'chrome-extension://old-ext/bg.html' },
        { id: 'sw-1', type: 'service_worker', title: 'MV3 Ext', url: 'chrome-extension://ext-1/sw.js' },
      ],
    })

    const result = await listMessageableExtensions(backend)

    expect(result).toHaveLength(1)
    expect(result[0]).toMatchObject({
      id: 'ext-1',
      type: 'service_worker',
    })
  })
})

// ── Zone 3: Multi-flag ──

describe('extension-bridge — Zone 3: multi-flag', () => {
  test('deriveScopeURL derives correct scope from various script URLs', () => {
    expect(deriveScopeURL('chrome-extension://abc123/sw.js')).toBe('chrome-extension://abc123/')
    expect(deriveScopeURL('chrome-extension://abc123/scripts/worker.js')).toBe('chrome-extension://abc123/')
    expect(deriveScopeURL('chrome-extension://xyz/sub/bg.js')).toBe('chrome-extension://xyz/')
  })

  test('listMessageableExtensions handles extensions with both SW and page targets', async () => {
    const backend = createMockBackend({
      getTargets: [
        { id: 'page-1', type: 'page', title: 'Ext popup', url: 'chrome-extension://ext-1/popup.html' },
        { id: 'sw-1', type: 'service_worker', title: 'Ext SW', url: 'chrome-extension://ext-1/sw.js' },
      ],
    })

    const result = await listMessageableExtensions(backend)

    // Should only return the SW target
    expect(result).toHaveLength(1)
    expect(result[0].type).toBe('service_worker')
  })
})

// ── Happy path ──

describe('extension-bridge — happy path', () => {
  test('listMessageableExtensions returns extensions with active SWs', async () => {
    const backend = createMockBackend({
      getTargets: [
        {
          id: 'sw-1',
          type: 'service_worker',
          title: 'uBlock SW',
          url: 'chrome-extension://cjpalhdlnbpafiamejdnhcphjbkeiagm/background.js',
        },
        {
          id: 'sw-2',
          type: 'service_worker',
          title: 'My Ext SW',
          url: 'chrome-extension://myext123/sw.js',
        },
      ],
    })

    const result = await listMessageableExtensions(backend)

    expect(result).toHaveLength(2)
    expect(result[0]).toMatchObject({
      id: 'cjpalhdlnbpafiamejdnhcphjbkeiagm',
      name: 'uBlock SW',
      type: 'service_worker',
      hasActiveBackground: true,
    })
    expect(result[1]).toMatchObject({
      id: 'myext123',
      name: 'My Ext SW',
      type: 'service_worker',
      hasActiveBackground: true,
    })
  })

  test('sendExtensionMessage sends message and returns response', async () => {
    const backend = createMockBackend({
      getTargets: [
        {
          id: 'sw-1',
          type: 'service_worker',
          title: 'Test Ext',
          url: 'chrome-extension://ext-1/sw.js',
        },
      ],
      evaluateResult: {
        result: {
          type: 'string',
          value: JSON.stringify({ status: 'ok', data: 42 }),
        },
      },
    })

    const result = await sendExtensionMessage(backend, 'ext-1', {
      action: 'ping',
    })

    expect(result).toEqual({ status: 'ok', data: 42 })
  })

  test('sendExtensionMessage returns null when extension has no onMessage listener', async () => {
    const backend = createMockBackend({
      getTargets: [
        {
          id: 'sw-1',
          type: 'service_worker',
          title: 'Silent Ext',
          url: 'chrome-extension://ext-1/sw.js',
        },
      ],
      evaluateResult: {
        result: {
          type: 'undefined',
        },
      },
    })

    const result = await sendExtensionMessage(backend, 'ext-1', {
      action: 'ping',
    })

    expect(result).toBeNull()
  })
})
