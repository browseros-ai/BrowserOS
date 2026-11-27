import { afterEach, describe, expect, it } from 'bun:test'
import type { spawnSync } from 'node:child_process'
import { resetMacOSProxyCache } from '../../../src/lib/proxy/macos-proxy'
import { resetPacCache } from '../../../src/lib/proxy/pac-evaluator'
import { resolveProxyForUrl } from '../../../src/lib/proxy/resolve-proxy'
import { resetWindowsProxyCache } from '../../../src/lib/proxy/windows-registry'

type RunFn = typeof spawnSync

function stubRun(stdout: string): RunFn {
  return (() => ({ stdout })) as unknown as RunFn
}

const REG_MANUAL = `HKEY_CURRENT_USER\\Software\\Microsoft\\Windows\\CurrentVersion\\Internet Settings
    ProxyEnable    REG_DWORD    0x1
    ProxyServer    REG_SZ    http=proxy.corp:8080;https=proxy.corp:8443
    ProxyOverride    REG_SZ    *.local;<local>
`

const REG_PAC = `HKEY_CURRENT_USER\\Software\\Microsoft\\Windows\\CurrentVersion\\Internet Settings
    ProxyEnable    REG_DWORD    0x1
    ProxyServer    REG_SZ    proxy.corp:8080
    AutoConfigURL    REG_SZ    http://wpad.corp/wpad.dat
`

const SCUTIL = `<dictionary> {
  ExceptionsList : <array> {
    0 : *.local
  }
  ExcludeSimpleHostnames : 1
  HTTPEnable : 1
  HTTPPort : 8080
  HTTPProxy : proxy.corp
  HTTPSEnable : 0
}
`

afterEach(() => {
  resetWindowsProxyCache()
  resetMacOSProxyCache()
  resetPacCache()
})

describe('resolveProxyForUrl', () => {
  it('returns undefined for non-http URLs', async () => {
    await expect(
      resolveProxyForUrl('not-a-url', { env: {} }),
    ).resolves.toBeUndefined()
  })

  it('prefers the env proxy over system settings', async () => {
    await expect(
      resolveProxyForUrl('https://api.openai.com/v1', {
        platform: 'win32',
        env: { HTTPS_PROXY: 'http://env-proxy:8080' } as NodeJS.ProcessEnv,
        run: stubRun(REG_MANUAL),
      }),
    ).resolves.toBe('http://env-proxy:8080')
  })

  it('honors NO_PROXY for env proxies', async () => {
    await expect(
      resolveProxyForUrl('https://internal.corp/x', {
        platform: 'linux',
        env: {
          HTTPS_PROXY: 'http://env-proxy:8080',
          NO_PROXY: '.corp',
        } as NodeJS.ProcessEnv,
      }),
    ).resolves.toBeUndefined()
  })

  it('always bypasses local LLM servers', async () => {
    await expect(
      resolveProxyForUrl('http://127.0.0.1:11434/v1/models', {
        platform: 'linux',
        env: { HTTP_PROXY: 'http://env-proxy:8080' } as NodeJS.ProcessEnv,
      }),
    ).resolves.toBeUndefined()
  })

  it('reads the Windows manual proxy per scheme', async () => {
    const run = stubRun(REG_MANUAL)
    await expect(
      resolveProxyForUrl('https://api.openai.com/v1', {
        platform: 'win32',
        env: {},
        run,
      }),
    ).resolves.toBe('http://proxy.corp:8443')
    await expect(
      resolveProxyForUrl('http://example.com/', {
        platform: 'win32',
        env: {},
        run,
      }),
    ).resolves.toBe('http://proxy.corp:8080')
  })

  it('honors ProxyOverride on Windows', async () => {
    await expect(
      resolveProxyForUrl('https://app.local/x', {
        platform: 'win32',
        env: {},
        run: stubRun(REG_MANUAL),
      }),
    ).resolves.toBeUndefined()
    await expect(
      resolveProxyForUrl('http://ollama/x', {
        platform: 'win32',
        env: {},
        run: stubRun(REG_MANUAL),
      }),
    ).resolves.toBeUndefined()
  })

  it('evaluates the PAC before falling back to the manual entry', async () => {
    const calls: Array<[string, string]> = []
    const evaluatePac = async (pacUrl: string, target: string) => {
      calls.push([pacUrl, target])
      return { kind: 'proxy', url: 'http://pac-proxy:3128' } as const
    }
    await expect(
      resolveProxyForUrl('https://api.openai.com/v1', {
        platform: 'win32',
        env: {},
        run: stubRun(REG_PAC),
        evaluatePac,
      }),
    ).resolves.toBe('http://pac-proxy:3128')
    expect(calls[0][0]).toBe('http://wpad.corp/wpad.dat')
  })

  it('honors an explicit PAC DIRECT verdict without manual fallback', async () => {
    await expect(
      resolveProxyForUrl('https://api.openai.com/v1', {
        platform: 'win32',
        env: {},
        run: stubRun(REG_PAC),
        evaluatePac: async () => ({ kind: 'direct' }) as const,
      }),
    ).resolves.toBeUndefined()
  })

  it('falls back to the manual entry when PAC evaluation fails', async () => {
    await expect(
      resolveProxyForUrl('https://api.openai.com/v1', {
        platform: 'win32',
        env: {},
        run: stubRun(REG_PAC),
        evaluatePac: async () => ({ kind: 'failed' }) as const,
      }),
    ).resolves.toBe('http://proxy.corp:8080')
  })

  it('reads the macOS scutil proxy and exceptions', async () => {
    const run = stubRun(SCUTIL)
    await expect(
      resolveProxyForUrl('http://example.com/', {
        platform: 'darwin',
        env: {},
        run,
      }),
    ).resolves.toBe('http://proxy.corp:8080')
    await expect(
      resolveProxyForUrl('http://app.local/', {
        platform: 'darwin',
        env: {},
        run,
      }),
    ).resolves.toBeUndefined()
  })

  it('returns undefined on Linux with an empty env', async () => {
    await expect(
      resolveProxyForUrl('https://api.openai.com/v1', {
        platform: 'linux',
        env: {},
      }),
    ).resolves.toBeUndefined()
  })
})
