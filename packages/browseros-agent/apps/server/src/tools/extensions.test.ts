/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Tests for tools/extensions.ts — Agent-facing extension management tools
 *
 * TDD order (worst-first):
 *   Zone 4: CDP errors propagated through tool handler
 *   Zone 6: protected extension rejection
 *   Zone 1: empty extension list
 *   Happy path
 */

import { afterEach, beforeEach, describe, expect, mock, test } from 'bun:test'
import type { ToolContext, ToolDefinition } from './framework'
import type { ToolResponse } from './response'
import type { Browser } from '../browser/browser'
import type { CdpBackend, CdpTarget } from '../browser/backends/types'

// Import all tools
import {
  list_extensions,
  get_extension_info,
  enable_extension,
  disable_extension,
  uninstall_extension,
  load_unpacked_extension,
  get_extension_storage,
  set_extension_storage,
  clear_extension_storage,
  remove_extension_storage,
  send_extension_message,
  list_messageable_extensions,
} from './extensions'

// ── Helpers ──

function createMockResponse(): ToolResponse & {
  textParts: string[]
  dataParts: unknown[]
} {
  return {
    textParts: [],
    dataParts: [],
    text(msg: string) {
      this.textParts.push(msg)
    },
    data(d: unknown) {
      this.dataParts.push(d)
    },
  } as unknown as ToolResponse
}

function createMockBrowser(overrides?: Partial<Browser>): Browser {
  return {
    loadUnpackedExtension: mock(async (_path: string) => 'ext-123'),
    uninstallExtension: mock(async (_id: string) => {}),
    getExtensionStorage: mock(async () => ({})),
    setExtensionStorage: mock(async () => {}),
    removeExtensionStorage: mock(async () => {}),
    clearExtensionStorage: mock(async () => {}),
    sendExtensionMessage: mock(async () => null),
    listMessageableExtensions: mock(async () => []),
    isCdpConnected: mock(() => true),
    ...overrides,
  } as unknown as Browser
}

function createMockContext(browser?: Partial<Browser>): ToolContext {
  return {
    browser: createMockBrowser(browser),
    directories: { workingDir: '/tmp' },
  }
}

// ── Zone 4: Error propagation ──

describe('extension tools — Zone 4: error propagation', () => {
  test('load_unpacked_extension tool propagates CDP error', async () => {
    const ctx = createMockContext({
      loadUnpackedExtension: mock(async () => {
        throw new Error('Extension not found at path /bad')
      }),
    })
    const resp = createMockResponse()

    await expect(
      load_unpacked_extension.handler({ path: '/bad' }, ctx, resp),
    ).rejects.toThrow('Extension not found')
  })

  test('uninstall_extension tool propagates protected extension error', async () => {
    const ctx = createMockContext({
      uninstallExtension: mock(async () => {
        throw new Error('Cannot uninstall BrowserOS first-party extension')
      }),
    })
    const resp = createMockResponse()

    await expect(
      uninstall_extension.handler(
        { extensionId: 'bflpfmnmnokmjhmgnolecpppdbdophmk' },
        ctx,
        resp,
      ),
    ).rejects.toThrow('first-party')
  })
})

// ── Zone 1: Empty results ──

describe('extension tools — Zone 1: empty results', () => {
  test('load_unpacked_extension delegates to browser (no direct validation)', async () => {
    const loadMock = mock(async () => 'ext-id')
    const ctx = createMockContext({
      loadUnpackedExtension: loadMock,
    })
    const resp = createMockResponse()

    // Tool delegates to browser — browser module validates
    await load_unpacked_extension.handler({ path: '' }, ctx, resp)
    expect(loadMock).toHaveBeenCalledWith('')
  })
})

// ── Happy path ──

describe('extension tools — happy path', () => {
  test('load_unpacked_extension returns success with extension ID', async () => {
    const ctx = createMockContext({
      loadUnpackedExtension: mock(async () => 'ext-abc'),
    })
    const resp = createMockResponse()

    await load_unpacked_extension.handler({ path: '/my/ext' }, ctx, resp)

    expect(resp.dataParts[0]).toEqual({
      action: 'load_unpacked',
      id: 'ext-abc',
      path: '/my/ext',
    })
    expect(resp.textParts[0]).toContain('ext-abc')
  })

  test('uninstall_extension returns success for third-party extension', async () => {
    const uninstallMock = mock(async () => {})
    const ctx = createMockContext({
      uninstallExtension: uninstallMock,
    })
    const resp = createMockResponse()

    await uninstall_extension.handler(
      { extensionId: 'third-party-ext' },
      ctx,
      resp,
    )

    expect(resp.dataParts[0]).toEqual({
      action: 'uninstall',
      extensionId: 'third-party-ext',
    })
  })

  test('get_extension_storage returns storage data', async () => {
    const ctx = createMockContext({
      getExtensionStorage: mock(async () => ({ key1: 'val1', key2: 42 })),
    })
    const resp = createMockResponse()

    await get_extension_storage.handler(
      { extensionId: 'ext-1', storageArea: 'local' },
      ctx,
      resp,
    )

    expect(resp.dataParts[0]).toMatchObject({
      action: 'get_storage',
      data: { key1: 'val1', key2: 42 },
    })
  })

  test('set_extension_storage calls through', async () => {
    const setMock = mock(async () => {})
    const ctx = createMockContext({
      setExtensionStorage: setMock,
    })
    const resp = createMockResponse()

    await set_extension_storage.handler(
      {
        extensionId: 'ext-1',
        storageArea: 'sync',
        values: { pref: true },
      },
      ctx,
      resp,
    )

    expect(setMock).toHaveBeenCalledWith('ext-1', 'sync', { pref: true })
    expect(resp.dataParts[0]).toMatchObject({ action: 'set_storage' })
  })

  test('remove_extension_storage calls through', async () => {
    const removeMock = mock(async () => {})
    const ctx = createMockContext({
      removeExtensionStorage: removeMock,
    })
    const resp = createMockResponse()

    await remove_extension_storage.handler(
      { extensionId: 'ext-1', storageArea: 'local', keys: ['old'] },
      ctx,
      resp,
    )

    expect(removeMock).toHaveBeenCalledWith('ext-1', 'local', ['old'])
    expect(resp.dataParts[0]).toMatchObject({ action: 'remove_storage' })
  })

  test('clear_extension_storage calls through', async () => {
    const clearMock = mock(async () => {})
    const ctx = createMockContext({
      clearExtensionStorage: clearMock,
    })
    const resp = createMockResponse()

    await clear_extension_storage.handler(
      { extensionId: 'ext-1', storageArea: 'local' },
      ctx,
      resp,
    )

    expect(clearMock).toHaveBeenCalledWith('ext-1', 'local')
    expect(resp.dataParts[0]).toMatchObject({ action: 'clear_storage' })
  })
})

// ── F9: Stub tools must throw, not return fake data ──

describe('extension tools — F9: stub tools throw not fake-success', () => {
  test('get_extension_info stub throws "not yet implemented"', async () => {
    const ctx = createMockContext()
    const resp = createMockResponse()

    await expect(
      get_extension_info.handler({ extensionId: 'ext-123' }, ctx, resp),
    ).rejects.toThrow('not yet implemented')
  })

  test('enable_extension stub throws "not yet implemented"', async () => {
    const ctx = createMockContext()
    const resp = createMockResponse()

    await expect(
      enable_extension.handler({ extensionId: 'ext-123' }, ctx, resp),
    ).rejects.toThrow('not yet implemented')
  })

  test('disable_extension stub throws "not yet implemented"', async () => {
    const ctx = createMockContext()
    const resp = createMockResponse()

    await expect(
      disable_extension.handler({ extensionId: 'ext-123' }, ctx, resp),
    ).rejects.toThrow('not yet implemented')
  })
})

// ── F11: send_extension_message and list_messageable_extensions tests ──

describe('extension tools — F11: send_extension_message', () => {
  test('happy path — sends message and returns response', async () => {
    const sendMock = mock(async () => ({ status: 'ok', data: 42 }))
    const ctx = createMockContext({
      sendExtensionMessage: sendMock,
    })
    const resp = createMockResponse()

    await send_extension_message.handler(
      { extensionId: 'ext-1', message: { action: 'ping' } },
      ctx,
      resp,
    )

    expect(sendMock).toHaveBeenCalledWith('ext-1', { action: 'ping' }, undefined)
    expect(resp.dataParts[0]).toEqual({
      extensionId: 'ext-1',
      response: { status: 'ok', data: 42 },
    })
    expect(resp.textParts[0]).toContain('ext-1')
  })

  test('null response from extension', async () => {
    const sendMock = mock(async () => null)
    const ctx = createMockContext({
      sendExtensionMessage: sendMock,
    })
    const resp = createMockResponse()

    await send_extension_message.handler(
      { extensionId: 'ext-1', message: { action: 'ping' } },
      ctx,
      resp,
    )

    expect(resp.dataParts[0]).toEqual({
      extensionId: 'ext-1',
      response: null,
    })
  })

  test('timeout parameter passthrough', async () => {
    const sendMock = mock(async () => 'done')
    const ctx = createMockContext({
      sendExtensionMessage: sendMock,
    })
    const resp = createMockResponse()

    await send_extension_message.handler(
      { extensionId: 'ext-1', message: 'ping', timeout: 5000 },
      ctx,
      resp,
    )

    expect(sendMock).toHaveBeenCalledWith('ext-1', 'ping', 5000)
  })

  test('error propagation — browser throws', async () => {
    const ctx = createMockContext({
      sendExtensionMessage: mock(async () => {
        throw new Error('No discoverable target')
      }),
    })
    const resp = createMockResponse()

    await expect(
      send_extension_message.handler(
        { extensionId: 'ext-nope', message: {} },
        ctx,
        resp,
      ),
    ).rejects.toThrow('No discoverable target')
  })
})

describe('extension tools — F11: list_messageable_extensions', () => {
  test('happy path — returns extensions with active SWs', async () => {
    const ctx = createMockContext({
      listMessageableExtensions: mock(async () => [
        {
          id: 'ext-1',
          name: 'Test Extension',
          type: 'service_worker',
          hasActiveBackground: true,
          targetId: 't1',
          url: 'chrome-extension://ext-1/sw.js',
        },
      ]),
    })
    const resp = createMockResponse()

    await list_messageable_extensions.handler({}, ctx, resp)

    expect(resp.dataParts[0]).toMatchObject({
      count: 1,
      extensions: [{ id: 'ext-1', name: 'Test Extension', type: 'service_worker' }],
    })
    expect(resp.textParts[0]).toContain('1 messageable')
  })

  test('empty result — no extensions with active SWs', async () => {
    const ctx = createMockContext({
      listMessageableExtensions: mock(async () => []),
    })
    const resp = createMockResponse()

    await list_messageable_extensions.handler({}, ctx, resp)

    expect(resp.dataParts[0]).toEqual({ extensions: [], count: 0 })
    expect(resp.textParts[0]).toContain('No extensions')
  })
})

// ── F17: remove_extension_storage rejects empty keys array ──

describe('extension tools — F17: input validation', () => {
  test('remove_extension_storage rejects empty keys array via schema', () => {
    // Zod schema should reject keys: []
    const schema = remove_extension_storage.input
    const result = schema.safeParse({
      extensionId: 'ext-1',
      storageArea: 'local',
      keys: [],
    })
    expect(result.success).toBe(false)
  })

  test('remove_extension_storage accepts non-empty keys array', () => {
    const schema = remove_extension_storage.input
    const result = schema.safeParse({
      extensionId: 'ext-1',
      storageArea: 'local',
      keys: ['key1'],
    })
    expect(result.success).toBe(true)
  })
})

// ── F18: send_extension_message timeout bounds ──

describe('extension tools — F18: timeout bounds validation', () => {
  test('timeout below 100ms is rejected', () => {
    const schema = send_extension_message.input
    const result = schema.safeParse({
      extensionId: 'ext-1',
      message: {},
      timeout: 50,
    })
    expect(result.success).toBe(false)
  })

  test('timeout above 30000ms is rejected', () => {
    const schema = send_extension_message.input
    const result = schema.safeParse({
      extensionId: 'ext-1',
      message: {},
      timeout: 60000,
    })
    expect(result.success).toBe(false)
  })

  test('timeout 10000ms (default) is accepted', () => {
    const schema = send_extension_message.input
    const result = schema.safeParse({
      extensionId: 'ext-1',
      message: {},
      timeout: 10000,
    })
    expect(result.success).toBe(true)
  })

  test('no timeout (undefined) is accepted', () => {
    const schema = send_extension_message.input
    const result = schema.safeParse({
      extensionId: 'ext-1',
      message: {},
    })
    expect(result.success).toBe(true)
  })
})

// ── Additional: managed area + keys forwarding + error propagation ──

describe('extension tools — additional coverage', () => {
  test('get_extension_storage accepts managed area (read-only)', async () => {
    const getMock = mock(async () => ({ policy: 'value' }))
    const ctx = createMockContext({
      getExtensionStorage: getMock,
    })
    const resp = createMockResponse()

    await get_extension_storage.handler(
      { extensionId: 'ext-1', storageArea: 'managed' },
      ctx,
      resp,
    )

    expect(getMock).toHaveBeenCalledWith('ext-1', 'managed', undefined)
    expect(resp.dataParts[0]).toMatchObject({
      action: 'get_storage',
      storageArea: 'managed',
      data: { policy: 'value' },
    })
  })

  test('get_extension_storage forwards keys parameter', async () => {
    const getMock = mock(async () => ({ key1: 'val1' }))
    const ctx = createMockContext({
      getExtensionStorage: getMock,
    })
    const resp = createMockResponse()

    await get_extension_storage.handler(
      { extensionId: 'ext-1', storageArea: 'local', keys: ['key1'] },
      ctx,
      resp,
    )

    expect(getMock).toHaveBeenCalledWith('ext-1', 'local', ['key1'])
  })

  test('set_extension_storage propagates CDP error', async () => {
    const ctx = createMockContext({
      setExtensionStorage: mock(async () => {
        throw new Error('CDP disconnected')
      }),
    })
    const resp = createMockResponse()

    await expect(
      set_extension_storage.handler(
        { extensionId: 'ext-1', storageArea: 'local', values: { a: 1 } },
        ctx,
        resp,
      ),
    ).rejects.toThrow('CDP disconnected')
  })

  test('remove_extension_storage propagates CDP error', async () => {
    const ctx = createMockContext({
      removeExtensionStorage: mock(async () => {
        throw new Error('Extension not found')
      }),
    })
    const resp = createMockResponse()

    await expect(
      remove_extension_storage.handler(
        { extensionId: 'ext-1', storageArea: 'local', keys: ['key1'] },
        ctx,
        resp,
      ),
    ).rejects.toThrow('Extension not found')
  })

  test('clear_extension_storage propagates CDP error', async () => {
    const ctx = createMockContext({
      clearExtensionStorage: mock(async () => {
        throw new Error('First-party protected')
      }),
    })
    const resp = createMockResponse()

    await expect(
      clear_extension_storage.handler(
        { extensionId: 'ext-1', storageArea: 'local' },
        ctx,
        resp,
      ),
    ).rejects.toThrow('First-party protected')
  })

  test('get_extension_storage propagates CDP error', async () => {
    const ctx = createMockContext({
      getExtensionStorage: mock(async () => {
        throw new Error('Extension not found')
      }),
    })
    const resp = createMockResponse()

    await expect(
      get_extension_storage.handler(
        { extensionId: 'ext-1', storageArea: 'local' },
        ctx,
        resp,
      ),
    ).rejects.toThrow('Extension not found')
  })
})
