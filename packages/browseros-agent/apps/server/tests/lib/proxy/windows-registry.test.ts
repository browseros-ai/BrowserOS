import { afterEach, describe, expect, it } from 'bun:test'
import type { spawnSync } from 'node:child_process'
import {
  parseProxyOverride,
  parseProxyServer,
  parseRegQueryOutput,
  readWindowsProxySettings,
  resetWindowsProxyCache,
} from '../../../src/lib/proxy/windows-registry'

type RunFn = typeof spawnSync

function stubRun(stdout: string): { run: RunFn; calls: () => number } {
  let calls = 0
  const run = ((_command: string, _args?: readonly string[]) => {
    calls += 1
    return { stdout } as ReturnType<RunFn>
  }) as RunFn
  return { run, calls: () => calls }
}

const MANUAL_OUTPUT = `HKEY_CURRENT_USER\\Software\\Microsoft\\Windows\\CurrentVersion\\Internet Settings
    ProxyEnable    REG_DWORD    0x1
    ProxyServer    REG_SZ    http=proxy.corp:8080;https=proxy.corp:8443
    ProxyOverride    REG_SZ    *.local;<local>
`

const PAC_OUTPUT = `HKEY_CURRENT_USER\\Software\\Microsoft\\Windows\\CurrentVersion\\Internet Settings
    ProxyEnable    REG_DWORD    0x0
    AutoConfigURL    REG_SZ    http://wpad.corp/wpad.dat
`

afterEach(() => {
  resetWindowsProxyCache()
})

describe('readWindowsProxySettings', () => {
  it('returns undefined off Windows without spawning', () => {
    const { run, calls } = stubRun(MANUAL_OUTPUT)
    expect(readWindowsProxySettings({ platform: 'linux', run })).toBeUndefined()
    expect(calls()).toBe(0)
  })

  it('reads manual settings and caches the result', () => {
    const { run, calls } = stubRun(MANUAL_OUTPUT)
    expect(readWindowsProxySettings({ platform: 'win32', run })).toEqual({
      enabled: true,
      proxyServer: 'http=proxy.corp:8080;https=proxy.corp:8443',
      proxyOverride: '*.local;<local>',
      autoConfigUrl: undefined,
    })
    expect(readWindowsProxySettings({ platform: 'win32', run })).toEqual(
      expect.objectContaining({ enabled: true }),
    )
    expect(calls()).toBe(1)
  })

  it('returns undefined when reg fails', () => {
    const run = (() => {
      throw new Error('boom')
    }) as RunFn
    expect(readWindowsProxySettings({ platform: 'win32', run })).toBeUndefined()
  })
})

describe('parseRegQueryOutput', () => {
  it('parses a PAC-only configuration', () => {
    expect(parseRegQueryOutput(PAC_OUTPUT)).toEqual({
      enabled: false,
      proxyServer: undefined,
      proxyOverride: undefined,
      autoConfigUrl: 'http://wpad.corp/wpad.dat',
    })
  })

  it('returns undefined for empty output', () => {
    expect(parseRegQueryOutput('')).toBeUndefined()
  })
})

describe('parseProxyServer', () => {
  it('parses per-scheme entries', () => {
    expect(
      parseProxyServer('http=proxy.corp:8080;https=proxy.corp:8443'),
    ).toEqual({
      http: 'http://proxy.corp:8080',
      https: 'http://proxy.corp:8443',
    })
  })

  it('applies a bare entry to both schemes', () => {
    expect(parseProxyServer('proxy.corp:8080')).toEqual({
      http: 'http://proxy.corp:8080',
      https: 'http://proxy.corp:8080',
    })
  })

  it('drops SOCKS entries Bun cannot use', () => {
    expect(parseProxyServer('socks=socks.corp:1080')).toEqual({
      http: undefined,
      https: undefined,
    })
  })

  it('returns empty manual settings for missing values', () => {
    expect(parseProxyServer(undefined)).toEqual({
      http: undefined,
      https: undefined,
    })
  })
})

describe('parseProxyOverride', () => {
  it('splits on semicolons', () => {
    expect(parseProxyOverride('*.local;<local>;10.0.0.1')).toEqual([
      '*.local',
      '<local>',
      '10.0.0.1',
    ])
  })

  it('returns an empty list for missing values', () => {
    expect(parseProxyOverride(undefined)).toEqual([])
  })
})
