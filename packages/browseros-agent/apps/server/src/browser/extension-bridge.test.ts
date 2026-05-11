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

  // ── Fix F12: Verify detachFromTarget called on success ──

  test('sendExtensionMessage detaches session after successful send', async () => {
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
          value: JSON.stringify({ ok: true }),
        },
      },
    })

    await sendExtensionMessage(backend, 'ext-1', { action: 'ping' })

    expect(backend.Target.detachFromTarget).toHaveBeenCalledWith({
      sessionId: 'sw-sess-1',
    })
  })

  // ── Fix F13: Exhausted retries ──

  test('sendExtensionMessage throws after exhausting all retries', async () => {
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

    // Always fail with retriable error
    backend.Target.attachToTarget = mock(async () => {
      throw new Error('Target closed')
    })

    await expect(
      sendExtensionMessage(backend, 'ext-1', { action: 'test' }),
    ).rejects.toThrow('Target closed')

    // Should have attempted MAX_RETRIES times
    const attachCallCount = (backend.Target.attachToTarget as ReturnType<typeof mock>).mock.calls.length
    expect(attachCallCount).toBe(2)
  })

  // ── Fix F14: exceptionDetails in Runtime.evaluate result ──

  test('sendExtensionMessage throws when Runtime.evaluate returns exceptionDetails', async () => {
    const sessionApi = {
      Runtime: {
        enable: mock(async () => {}),
        evaluate: mock(async () => ({
          result: { type: 'object', value: {} },
          exceptionDetails: { text: 'Extension context was destroyed' },
        })),
        on: mock(() => {}),
      },
    } as unknown as ProtocolApi

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
    // Override session to return exceptionDetails
    backend.session = mock((() => sessionApi) as any)

    await expect(
      sendExtensionMessage(backend, 'ext-1', { action: 'test' }),
    ).rejects.toThrow('Extension context was destroyed')
  })

  // ── Fix F15: Timeout behavior ──

  test('sendExtensionMessage throws timeout error when extension does not respond', async () => {
    // Create a session where evaluate never resolves
    let rejectEvaluate: (err: Error) => void
    const evaluatePromise = new Promise<any>((_resolve, reject) => {
      rejectEvaluate = reject
    })
    const sessionApi = {
      Runtime: {
        enable: mock(async () => {}),
        evaluate: mock(async () => evaluatePromise),
        on: mock(() => {}),
      },
    } as unknown as ProtocolApi

    const backend = createMockBackend({
      getTargets: [
        {
          id: 'sw-1',
          type: 'service_worker',
          title: 'Slow Ext',
          url: 'chrome-extension://ext-1/sw.js',
        },
      ],
    })
    backend.session = mock((() => sessionApi) as any)

    // Use very short timeout
    await expect(
      sendExtensionMessage(backend, 'ext-1', { action: 'slow' }, 50),
    ).rejects.toThrow('timeout')
  })

  // ── Fix F16: Non-serializable message ──

  test('sendExtensionMessage throws descriptive error for non-serializable payload', async () => {
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

    // Circular reference
    const circular: any = { a: 1 }
    circular.self = circular

    await expect(
      sendExtensionMessage(backend, 'ext-1', circular),
    ).rejects.toThrow('not JSON-serializable')
  })
})

// ── Fix F1: Session leak on error path ──

describe('extension-bridge — F1: session cleanup on error', () => {
  test('sendExtensionMessage detaches session when Runtime.evaluate throws', async () => {
    const sessionApi = {
      Runtime: {
        enable: mock(async () => {}),
        evaluate: mock(async () => {
          throw new Error('Context destroyed')
        }),
        on: mock(() => {}),
      },
    } as unknown as ProtocolApi

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
    backend.session = mock((() => sessionApi) as any)

    await expect(
      sendExtensionMessage(backend, 'ext-1', { action: 'test' }),
    ).rejects.toThrow('Context destroyed')

    // Critical: session must be detached even on error
    expect(backend.Target.detachFromTarget).toHaveBeenCalledWith({
      sessionId: 'sw-sess-1',
    })
  })
})

// ── Fix F8: Empty extensionId guard ──

describe('extension-bridge — F8: input validation', () => {
  test('sendExtensionMessage throws on empty extensionId', async () => {
    const backend = createMockBackend()

    await expect(
      sendExtensionMessage(backend, '', { action: 'test' }),
    ).rejects.toThrow('Extension ID is required')
  })
})

// ── Fix F4: Stale swTarget across retries ──

describe('extension-bridge — F4: re-discover targets after startWorker', () => {
  test('sendExtensionMessage re-discovers target after SW restart on retry', async () => {
    let getTargetsCallCount = 0
    let attachCallCount = 0

    const sessionApi = {
      Runtime: {
        enable: mock(async () => {}),
        evaluate: mock(async () => ({
          result: { type: 'string', value: JSON.stringify({ ok: true }) },
        })),
        on: mock(() => {}),
      },
    } as unknown as ProtocolApi

    const backend = createMockBackend()

    // First getTargets returns old target, second (after retry) returns new target
    backend.getTargets = mock(async () => {
      getTargetsCallCount++
      if (getTargetsCallCount === 1) {
        return [{ id: 'sw-old', type: 'service_worker', title: 'Old SW', url: 'chrome-extension://ext-1/sw.js' }]
      }
      return [{ id: 'sw-new', type: 'service_worker', title: 'New SW', url: 'chrome-extension://ext-1/sw.js' }]
    })

    // First attach to old target fails, second to new target succeeds
    backend.Target.attachToTarget = mock(async (params: any) => {
      attachCallCount++
      if (params.targetId === 'sw-old') throw new Error('Target closed')
      return { sessionId: 'sw-sess-new' }
    })

    backend.session = mock((() => sessionApi) as any)

    const result = await sendExtensionMessage(backend, 'ext-1', { action: 'test' })

    expect(result).toEqual({ ok: true })
    // Should have called getTargets at least twice (initial + retry re-discover)
    expect(getTargetsCallCount).toBeGreaterThanOrEqual(2)
    // Should have attached to the NEW target
    expect(backend.Target.attachToTarget).toHaveBeenCalledWith(
      expect.objectContaining({ targetId: 'sw-new' }),
    )
  })
})
