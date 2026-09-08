import { afterEach, describe, expect, it } from 'bun:test'
import { within } from '../src/collector'
import { REPORTER_EXTENSION_ID } from '../src/contract'
import { canReadDiagnostics, collectBrowserDiagnostics } from '../src/extension'

const original = globalThis.chrome
afterEach(() => {
  globalThis.chrome = original
})

describe('diagnostics browser boundary', () => {
  it('only accepts Reporter externally and extension pages internally', () => {
    expect(canReadDiagnostics({ id: REPORTER_EXTENSION_ID }, 'app', true)).toBe(
      true,
    )
    expect(canReadDiagnostics({ id: 'other' }, 'app', true)).toBe(false)
    expect(
      canReadDiagnostics({ url: 'https://browseros.com' }, 'app', true),
    ).toBe(false)
    expect(
      canReadDiagnostics(
        { id: 'app', url: 'https://example.com' },
        'app',
        false,
      ),
    ).toBe(false)
    expect(
      canReadDiagnostics(
        { id: 'app', url: 'chrome-extension://app/app.html' },
        'app',
        false,
      ),
    ).toBe(true)
  })
  it('maps both native version functions correctly when the server and Reporter fail', async () => {
    globalThis.chrome = {
      runtime: {
        getManifest: () => ({ version: 'extension-version' }),
        getPlatformInfo: async () => ({ os: 'mac', arch: 'arm64' }),
        sendMessage: async () => {
          throw new Error('Reporter missing')
        },
      },
      browserOS: {
        getVersionNumber: (callback: (value: string) => void) =>
          callback('chromium-version'),
        getBrowserosVersionNumber: (callback: (value: string) => void) =>
          callback('browseros-version'),
      },
      system: {
        cpu: {
          getInfo: async () => ({
            modelName: 'Example CPU',
            numOfProcessors: 8,
          }),
        },
        memory: {
          getInfo: async () => ({ capacity: 4096, availableCapacity: 2048 }),
        },
      },
    } as unknown as typeof chrome
    const snapshot = await collectBrowserDiagnostics('browseros', async () => {
      throw new Error('server unavailable')
    })
    expect(snapshot.versions).toEqual({
      browseros: 'browseros-version',
      chromium: 'chromium-version',
      server: null,
      appExtension: 'extension-version',
      reporterExtension: null,
    })
    expect(snapshot.system.os).toBe('macOS')
    expect(snapshot.system.osVersion).toBeNull()
    expect(snapshot.memory.availableBytes).toBe(2048)
  })
  it('bounds a source which never answers', async () => {
    await expect(within(() => new Promise(() => {}), 5)).rejects.toThrow(
      'timed out',
    )
  })
})
