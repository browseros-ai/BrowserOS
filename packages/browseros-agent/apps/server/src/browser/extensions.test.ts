/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Tests for browser/extensions.ts — Extension management via CDP
 *
 * TDD order (worst-first):
 *   Zone 4: error propagation (CDP failures, missing ext, protected ext)
 *   Zone 1: empty results (no extensions)
 *   Zone 6: permission boundaries (component ext, managed storage)
 *   Zone 3: multi-flag (enable-then-disable, storage after load)
 *   Happy path
 */

import { afterEach, beforeEach, describe, expect, mock, test } from 'bun:test'
import type { ProtocolApi } from '@browseros/cdp-protocol/protocol-api'
import type { CdpBackend, CdpTarget } from './backends/types'
import type { ExtensionInfo } from '@browseros/cdp-protocol/generated/domains/extensions'
import { Browser } from './browser'
import * as extensions from './extensions'

// ── Mock factories ──

function createMockCdpBackend(): CdpBackend {
  return {
    connect: mock(async () => {}),
    disconnect: mock(async () => {}),
    isConnected: mock(() => true),
    getTargets: mock(async () => [] as CdpTarget[]),
    session: mock(() => createMockSessionApi()),
    onSessionEvent: mock(() => () => {}),
    // ProtocolApi stubs
    Extensions: {
      loadUnpacked: mock(async () => ({ id: 'ext-123' })),
      uninstall: mock(async () => {}),
      getStorageItems: mock(async () => ({ data: {} })),
      setStorageItems: mock(async () => {}),
      removeStorageItems: mock(async () => {}),
      clearStorageItems: mock(async () => {}),
      listExtensions: mock(async () => ({ extensions: [] })),
      getExtensionInfo: mock(async () => ({ info: {} as any })),
      enableExtension: mock(async () => {}),
      disableExtension: mock(async () => {}),
    },
    // Minimum ProtocolApi stubs
    Target: {
      setAutoAttach: mock(async () => {}),
      on: mock(() => {}),
      getTargets: mock(async () => ({ targetInfos: [] })),
      attachToTarget: mock(async () => ({ sessionId: 'sess-1' })),
      detachFromTarget: mock(async () => {}),
      createTarget: mock(async () => ({ targetId: 't-1' })),
      closeTarget: mock(async () => ({ success: true })),
      activateTarget: mock(async () => {}),
    },
    Browser: {
      getWindowForTarget: mock(async () => ({
        windowId: 1,
        bounds: {},
      })),
      setWindowBounds: mock(async () => {}),
    },
    Page: {
      enable: mock(async () => {}),
      disable: mock(async () => {}),
      on: mock(() => {}),
      getFrameTree: mock(async () => ({
        frameTree: {
          frame: { id: 'f1', url: 'about:blank', loaderId: 'l1' },
        },
      })),
    },
    Runtime: {
      enable: mock(async () => {}),
      on: mock(() => {}),
      evaluate: mock(async () => ({
        result: { type: 'undefined' },
      })),
    },
    ServiceWorker: {
      enable: mock(async () => {}),
    },
    Emulation: {
      setDeviceMetricsOverride: mock(async () => {}),
    },
  } as unknown as CdpBackend
}

function createMockSessionApi(): ProtocolApi {
  return {
    Extensions: {
      loadUnpacked: mock(async () => ({ id: 'ext-123' })),
      uninstall: mock(async () => {}),
      getStorageItems: mock(async () => ({ data: {} })),
      setStorageItems: mock(async () => {}),
      removeStorageItems: mock(async () => {}),
      clearStorageItems: mock(async () => {}),
      listExtensions: mock(async () => ({ extensions: [] })),
      getExtensionInfo: mock(async () => ({ info: {} as any })),
      enableExtension: mock(async () => {}),
      disableExtension: mock(async () => {}),
    },
    Target: {
      setAutoAttach: mock(async () => {}),
      on: mock(() => {}),
      getTargets: mock(async () => ({ targetInfos: [] })),
      attachToTarget: mock(async () => ({ sessionId: 'sess-1' })),
      detachFromTarget: mock(async () => {}),
      createTarget: mock(async () => ({ targetId: 't-1' })),
      closeTarget: mock(async () => ({ success: true })),
      activateTarget: mock(async () => {}),
    },
    Page: {
      enable: mock(async () => {}),
      disable: mock(async () => {}),
      on: mock(() => {}),
      getFrameTree: mock(async () => ({
        frameTree: {
          frame: { id: 'f1', url: 'about:blank', loaderId: 'l1' },
        },
      })),
    },
    Runtime: {
      enable: mock(async () => {}),
      on: mock(() => {}),
      evaluate: mock(async () => ({
        result: { type: 'undefined' },
      })),
    },
    ServiceWorker: {
      enable: mock(async () => {}),
    },
    Emulation: {
      setDeviceMetricsOverride: mock(async () => {}),
    },
    Browser: {
      getWindowForTarget: mock(async () => ({
        windowId: 1,
        bounds: {},
      })),
      setWindowBounds: mock(async () => {}),
    },
  } as unknown as ProtocolApi
}

// ── Zone 4: Error Propagation ──

describe('extensions module — Zone 4: error propagation', () => {
  test('loadUnpacked throws on invalid path', async () => {
    const backend = createMockCdpBackend()
    backend.Extensions.loadUnpacked = mock(async () => {
      throw new Error('Extension not found at path /nonexistent')
    })

    await expect(
      extensions.loadUnpackedExtension(backend, '/nonexistent'),
    ).rejects.toThrow('Extension not found')
  })

  test('uninstallExtension throws on unknown extension ID', async () => {
    const backend = createMockCdpBackend()
    backend.Extensions.uninstall = mock(async () => {
      throw new Error('Extension not found: unknown-id')
    })

    await expect(
      extensions.uninstallExtension(backend, 'unknown-id'),
    ).rejects.toThrow('Extension not found')
  })

  test('getStorageItems DOES allow managed storage area (read-only)', async () => {
    const backend = createMockCdpBackend()
    backend.Extensions.getStorageItems = mock(async () => ({
      data: { policy: 'value' },
    }))

    // managed is read-only, so reads are allowed
    const result = await extensions.getStorageItems(backend, 'ext-id', 'managed')
    expect(result).toEqual({ policy: 'value' })
  })

  test('setStorageItems throws on managed storage area', async () => {
    const backend = createMockCdpBackend()

    await expect(
      extensions.setStorageItems(backend, 'ext-id', 'managed', { key: 'val' }),
    ).rejects.toThrow('managed')
  })

  test('removeStorageItems throws on managed storage area', async () => {
    const backend = createMockCdpBackend()

    await expect(
      extensions.removeStorageItems(backend, 'ext-id', 'managed', ['key']),
    ).rejects.toThrow('managed')
  })

  test('clearStorageItems throws on managed storage area', async () => {
    const backend = createMockCdpBackend()

    await expect(
      extensions.clearStorageItems(backend, 'ext-id', 'managed'),
    ).rejects.toThrow('managed')
  })
})

// ── Zone 6: Permission / access boundaries ──

describe('extensions module — Zone 6: protected extensions', () => {
  const PROTECTED_IDS = [
    'bflpfmnmnokmjhmgnolecpppdbdophmk', // Agent
    'adlpneommgkgeanpaekgoaolcpncohkf', // Bug Reporter
    'nlnihljpboknmfagkikhkdblbedophja', // Controller
  ]

  test.each(PROTECTED_IDS)(
    'uninstallExtension rejects BrowserOS first-party extension %s',
    async (id) => {
      const backend = createMockCdpBackend()

      await expect(
        extensions.uninstallExtension(backend, id),
      ).rejects.toThrow('first-party')
    },
  )

  test('isBrowserOSExtension returns true for first-party IDs', () => {
    expect(extensions.isBrowserOSExtension('bflpfmnmnokmjhmgnolecpppdbdophmk')).toBe(true)
    expect(extensions.isBrowserOSExtension('adlpneommgkgeanpaekgoaolcpncohkf')).toBe(true)
    expect(extensions.isBrowserOSExtension('nlnihljpboknmfagkikhkdblbedophja')).toBe(true)
  })

  test('isBrowserOSExtension returns false for third-party IDs', () => {
    expect(extensions.isBrowserOSExtension('cjpalhdlnbpafiamejdnhcphjbkeiagm')).toBe(false)
    expect(extensions.isBrowserOSExtension('random-extension-id')).toBe(false)
  })

  test.each(PROTECTED_IDS)(
    'setStorageItems rejects BrowserOS first-party extension %s',
    async (id) => {
      const backend = createMockCdpBackend()

      await expect(
        extensions.setStorageItems(backend, id, 'local', { key: 'val' }),
      ).rejects.toThrow('first-party')
    },
  )

  test.each(PROTECTED_IDS)(
    'removeStorageItems rejects BrowserOS first-party extension %s',
    async (id) => {
      const backend = createMockCdpBackend()

      await expect(
        extensions.removeStorageItems(backend, id, 'local', ['key']),
      ).rejects.toThrow('first-party')
    },
  )

  test.each(PROTECTED_IDS)(
    'clearStorageItems rejects BrowserOS first-party extension %s',
    async (id) => {
      const backend = createMockCdpBackend()

      await expect(
        extensions.clearStorageItems(backend, id, 'local'),
      ).rejects.toThrow('first-party')
    },
  )
})

// ── Zone 1: Empty / nil inputs ──

describe('extensions module — Zone 1: empty inputs', () => {
  test('loadUnpackedExtension throws on empty path', async () => {
    const backend = createMockCdpBackend()

    await expect(
      extensions.loadUnpackedExtension(backend, ''),
    ).rejects.toThrow()
  })

  test('getStorageItems with no keys returns all data', async () => {
    const backend = createMockCdpBackend()
    backend.Extensions.getStorageItems = mock(async () => ({
      data: { foo: 'bar', baz: 42 },
    }))

    const result = await extensions.getStorageItems(
      backend,
      'ext-id',
      'local',
    )

    expect(result).toEqual({ foo: 'bar', baz: 42 })
    expect(backend.Extensions.getStorageItems).toHaveBeenCalledWith({
      id: 'ext-id',
      storageArea: 'local',
    })
  })

  test('getStorageItems with specific keys returns filtered data', async () => {
    const backend = createMockCdpBackend()
    backend.Extensions.getStorageItems = mock(async () => ({
      data: { foo: 'bar' },
    }))

    const result = await extensions.getStorageItems(
      backend,
      'ext-id',
      'local',
      ['foo'],
    )

    expect(result).toEqual({ foo: 'bar' })
    expect(backend.Extensions.getStorageItems).toHaveBeenCalledWith({
      id: 'ext-id',
      storageArea: 'local',
      keys: ['foo'],
    })
  })

  test('getStorageItems returns empty object when CDP returns undefined data', async () => {
    const backend = createMockCdpBackend()
    backend.Extensions.getStorageItems = mock(async () => ({ data: undefined as any }))

    const result = await extensions.getStorageItems(backend, 'ext-id', 'local')

    expect(result).toEqual({})
  })
})

// ── Happy path ──

describe('extensions module — happy path', () => {
  test('loadUnpackedExtension calls CDP and returns id', async () => {
    const backend = createMockCdpBackend()
    backend.Extensions.loadUnpacked = mock(async () => ({
      id: 'new-ext-123',
    }))

    const result = await extensions.loadUnpackedExtension(
      backend,
      '/path/to/ext',
    )

    expect(result).toBe('new-ext-123')
    expect(backend.Extensions.loadUnpacked).toHaveBeenCalledWith({
      path: '/path/to/ext',
    })
  })

  test('uninstallExtension calls CDP for third-party extension', async () => {
    const backend = createMockCdpBackend()
    backend.Extensions.uninstall = mock(async () => {})

    await extensions.uninstallExtension(backend, 'third-party-ext-id')

    expect(backend.Extensions.uninstall).toHaveBeenCalledWith({
      id: 'third-party-ext-id',
    })
  })

  test('setStorageItems calls CDP with values', async () => {
    const backend = createMockCdpBackend()
    backend.Extensions.setStorageItems = mock(async () => {})

    await extensions.setStorageItems(backend, 'ext-id', 'sync', {
      key1: 'val1',
    })

    expect(backend.Extensions.setStorageItems).toHaveBeenCalledWith({
      id: 'ext-id',
      storageArea: 'sync',
      values: { key1: 'val1' },
    })
  })

  test('removeStorageItems calls CDP with keys', async () => {
    const backend = createMockCdpBackend()
    backend.Extensions.removeStorageItems = mock(async () => {})

    await extensions.removeStorageItems(backend, 'ext-id', 'local', [
      'key1',
      'key2',
    ])

    expect(backend.Extensions.removeStorageItems).toHaveBeenCalledWith({
      id: 'ext-id',
      storageArea: 'local',
      keys: ['key1', 'key2'],
    })
  })

  test('clearStorageItems calls CDP', async () => {
    const backend = createMockCdpBackend()
    backend.Extensions.clearStorageItems = mock(async () => {})

    await extensions.clearStorageItems(backend, 'ext-id', 'local')

    expect(backend.Extensions.clearStorageItems).toHaveBeenCalledWith({
      id: 'ext-id',
      storageArea: 'local',
    })
  })
})

// ── L2 Extension Management: list/getInfo/enable/disable ──

describe('extensions module — L2: listExtensions', () => {
  test('returns extensions array from CDP', async () => {
    const backend = createMockCdpBackend()
    const extList: ExtensionInfo[] = [
      {
        id: 'ext-1',
        name: 'Test Extension',
        version: '1.0.0',
        description: 'A test extension',
        path: '/path/to/ext',
        state: 'enabled',
        isBrowserOS: false,
        canModify: true,
      },
    ]
    backend.Extensions.listExtensions = mock(async () => ({ extensions: extList }))

    const result = await extensions.listExtensions(backend)

    expect(result).toEqual(extList)
    expect(result).toHaveLength(1)
  })

  test('returns empty array when no extensions', async () => {
    const backend = createMockCdpBackend()
    backend.Extensions.listExtensions = mock(async () => ({ extensions: [] }))

    const result = await extensions.listExtensions(backend)

    expect(result).toEqual([])
    expect(result).toHaveLength(0)
  })
})

describe('extensions module — L2: getExtensionInfo', () => {
  test('returns info for valid extension ID', async () => {
    const backend = createMockCdpBackend()
    const info: ExtensionInfo = {
      id: 'ext-1',
      name: 'Test Extension',
      version: '1.0.0',
      description: 'A test extension',
      path: '/path/to/ext',
      state: 'enabled',
      isBrowserOS: false,
      canModify: true,
    }
    backend.Extensions.getExtensionInfo = mock(async () => ({ info }))

    const result = await extensions.getExtensionInfo(backend, 'ext-1')

    expect(result).toEqual(info)
    expect(backend.Extensions.getExtensionInfo).toHaveBeenCalledWith({ id: 'ext-1' })
  })

  test('throws on empty ID', async () => {
    const backend = createMockCdpBackend()

    await expect(
      extensions.getExtensionInfo(backend, ''),
    ).rejects.toThrow('Extension ID is required')
  })

  test('propagates CDP error for unknown extension', async () => {
    const backend = createMockCdpBackend()
    backend.Extensions.getExtensionInfo = mock(async () => {
      throw new Error('Extension not found: unknown-id')
    })

    await expect(
      extensions.getExtensionInfo(backend, 'unknown-id'),
    ).rejects.toThrow('Extension not found')
  })
})

describe('extensions module — L2: enableExtension', () => {
  test('succeeds for non-first-party extension', async () => {
    const backend = createMockCdpBackend()
    backend.Extensions.enableExtension = mock(async () => {})

    await extensions.enableExtension(backend, 'third-party-ext')

    expect(backend.Extensions.enableExtension).toHaveBeenCalledWith({ id: 'third-party-ext' })
  })

  test('succeeds for first-party extension (may have been accidentally disabled)', async () => {
    const backend = createMockCdpBackend()
    backend.Extensions.enableExtension = mock(async () => {})

    // Enabling a first-party extension is fine — it might have been accidentally disabled
    await extensions.enableExtension(backend, 'bflpfmnmnokmjhmgnolecpppdbdophmk')

    expect(backend.Extensions.enableExtension).toHaveBeenCalledWith({ id: 'bflpfmnmnokmjhmgnolecpppdbdophmk' })
  })

  test('throws on empty ID', async () => {
    const backend = createMockCdpBackend()

    await expect(
      extensions.enableExtension(backend, ''),
    ).rejects.toThrow('Extension ID is required')
  })

  test('propagates CDP error', async () => {
    const backend = createMockCdpBackend()
    backend.Extensions.enableExtension = mock(async () => {
      throw new Error('Extension not found: missing')
    })

    await expect(
      extensions.enableExtension(backend, 'missing'),
    ).rejects.toThrow('Extension not found')
  })
})

describe('extensions module — L2: disableExtension', () => {
  test('succeeds for non-first-party extension', async () => {
    const backend = createMockCdpBackend()
    backend.Extensions.disableExtension = mock(async () => {})

    await extensions.disableExtension(backend, 'third-party-ext')

    expect(backend.Extensions.disableExtension).toHaveBeenCalledWith({ id: 'third-party-ext' })
  })

  test('throws on empty ID', async () => {
    const backend = createMockCdpBackend()

    await expect(
      extensions.disableExtension(backend, ''),
    ).rejects.toThrow('Extension ID is required')
  })

  test('throws on first-party BrowserOS extension', async () => {
    const backend = createMockCdpBackend()

    await expect(
      extensions.disableExtension(backend, 'bflpfmnmnokmjhmgnolecpppdbdophmk'),
    ).rejects.toThrow('first-party')
  })

  test('rejects all three BrowserOS first-party IDs', async () => {
    const backend = createMockCdpBackend()
    const ids = [
      'bflpfmnmnokmjhmgnolecpppdbdophmk',
      'adlpneommgkgeanpaekgoaolcpncohkf',
      'nlnihljpboknmfagkikhkdblbedophja',
    ]
    for (const id of ids) {
      await expect(
        extensions.disableExtension(backend, id),
      ).rejects.toThrow('first-party')
    }
  })

  test('propagates CDP error for non-first-party', async () => {
    const backend = createMockCdpBackend()
    backend.Extensions.disableExtension = mock(async () => {
      throw new Error('Extension already disabled')
    })

    await expect(
      extensions.disableExtension(backend, 'third-party'),
    ).rejects.toThrow('Extension already disabled')
  })
})
