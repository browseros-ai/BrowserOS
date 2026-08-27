import { afterEach, describe, expect, it } from 'bun:test'
import type { spawnSync } from 'node:child_process'
import {
  mergePath,
  resetLoginShellPathCache,
  resolveLoginShellPath,
} from '../../../../src/lib/agents/host-acp/resolve-login-path'

type RunFn = typeof spawnSync

function stubRun(stdout: string): { run: RunFn; calls: () => number } {
  let calls = 0
  const run = ((_command: string, _args?: readonly string[]) => {
    calls += 1
    return { stdout } as ReturnType<RunFn>
  }) as RunFn
  return { run, calls: () => calls }
}

afterEach(() => {
  resetLoginShellPathCache()
})

describe('resolveLoginShellPath', () => {
  it('returns undefined on Windows without spawning a shell', () => {
    const { run, calls } = stubRun('/should/not/run')
    expect(resolveLoginShellPath({ platform: 'win32', run })).toBeUndefined()
    expect(calls()).toBe(0)
  })

  it('returns the login-shell PATH on unix and caches it', () => {
    const { run, calls } = stubRun('/opt/homebrew/bin:/usr/bin\n')
    expect(resolveLoginShellPath({ platform: 'darwin', run })).toBe(
      '/opt/homebrew/bin:/usr/bin',
    )
    expect(resolveLoginShellPath({ platform: 'darwin', run })).toBe(
      '/opt/homebrew/bin:/usr/bin',
    )
    expect(calls()).toBe(1)
  })

  it('returns undefined when the login shell fails', () => {
    const run = (() => {
      throw new Error('boom')
    }) as RunFn
    expect(resolveLoginShellPath({ platform: 'linux', run })).toBeUndefined()
  })

  it('prefers the SHELL from the provided env', () => {
    let seenShell = ''
    const run = ((command: string) => {
      seenShell = command
      return { stdout: '/x' } as ReturnType<RunFn>
    }) as RunFn
    resolveLoginShellPath({
      platform: 'linux',
      env: { SHELL: '/bin/zsh' } as NodeJS.ProcessEnv,
      run,
    })
    expect(seenShell).toBe('/bin/zsh')
  })
})

describe('mergePath', () => {
  it('prepends the login PATH and de-dupes entries', () => {
    expect(mergePath('/a:/b', '/b:/c')).toBe('/a:/b:/c')
  })

  it('tolerates an empty inherited PATH', () => {
    expect(mergePath('/a:/b', '')).toBe('/a:/b')
  })
})
