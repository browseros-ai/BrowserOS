import { afterEach, describe, expect, it } from 'bun:test'
import type { spawnSync } from 'node:child_process'
import {
  parseScutilOutput,
  readMacOSProxySettings,
  resetMacOSProxyCache,
} from '../../../src/lib/proxy/macos-proxy'

type RunFn = typeof spawnSync

function stubRun(stdout: string): { run: RunFn; calls: () => number } {
  let calls = 0
  const run = ((_command: string, _args?: readonly string[]) => {
    calls += 1
    return { stdout } as ReturnType<RunFn>
  }) as RunFn
  return { run, calls: () => calls }
}

const SCUTIL_OUTPUT = `<dictionary> {
  ExceptionsList : <array> {
    0 : *.local
    1 : 169.254/16
  }
  ExcludeSimpleHostnames : 1
  FTPPassive : 1
  HTTPEnable : 1
  HTTPPort : 8080
  HTTPProxy : proxy.corp
  HTTPSEnable : 1
  HTTPSPort : 8443
  HTTPSProxy : proxy.corp
}
`

afterEach(() => {
  resetMacOSProxyCache()
})

describe('readMacOSProxySettings', () => {
  it('returns undefined off macOS without spawning', () => {
    const { run, calls } = stubRun(SCUTIL_OUTPUT)
    expect(readMacOSProxySettings({ platform: 'win32', run })).toBeUndefined()
    expect(calls()).toBe(0)
  })

  it('reads scutil output and caches the result', () => {
    const { run, calls } = stubRun(SCUTIL_OUTPUT)
    expect(readMacOSProxySettings({ platform: 'darwin', run })).toEqual({
      httpProxy: 'http://proxy.corp:8080',
      httpsProxy: 'http://proxy.corp:8443',
      exceptions: ['*.local', '169.254/16'],
      excludeSimpleHostnames: true,
    })
    readMacOSProxySettings({ platform: 'darwin', run })
    expect(calls()).toBe(1)
  })
})

describe('parseScutilOutput', () => {
  it('omits disabled proxies', () => {
    expect(parseScutilOutput('<dictionary> {\n  HTTPEnable : 0\n}\n')).toEqual({
      httpProxy: undefined,
      httpsProxy: undefined,
      exceptions: [],
      excludeSimpleHostnames: false,
    })
  })

  it('returns undefined for empty output', () => {
    expect(parseScutilOutput('')).toBeUndefined()
  })
})
