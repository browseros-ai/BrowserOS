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
