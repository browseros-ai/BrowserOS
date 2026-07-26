/**
 * @license
 * Copyright 2025 BrowserOS
 */

import { describe, expect, it, mock } from 'bun:test'
import {
  buildResolvedBinaryEnv,
  resolveHostBinary,
} from '../../../src/lib/agents/host-acp/binary-resolver'

describe('host binary resolver', () => {
  it('resolves Unix binaries through the user login shell first', async () => {
    const runCommand = mock(async () => ({
      exitCode: 0,
      stdout: '/Users/dev/.local/bin/claude\n',
      stderr: '',
    }))

    const result = await resolveHostBinary('claude', {
      env: { SHELL: '/bin/zsh', PATH: '/usr/bin:/bin' },
      platform: 'darwin',
      runCommand,
    })

    expect(result?.path).toBe('/Users/dev/.local/bin/claude')
    expect(runCommand).toHaveBeenCalledWith(
      '/bin/zsh',
      ['-lic', 'command -v claude'],
      expect.objectContaining({
        env: { SHELL: '/bin/zsh', PATH: '/usr/bin:/bin' },
      }),
    )
  })

  it('falls back to sh lookup on Unix when the login shell misses', async () => {
    const runCommand = mock(async (cmd: string) =>
      cmd === '/bin/fish'
        ? { exitCode: 1, stdout: '', stderr: 'not found' }
        : { exitCode: 0, stdout: '/usr/local/bin/codex\n', stderr: '' },
    )

    const result = await resolveHostBinary('codex', {
      env: { SHELL: '/bin/fish', PATH: '/usr/bin:/bin' },
      platform: 'linux',
      runCommand,
    })

    expect(result?.path).toBe('/usr/local/bin/codex')
    expect(runCommand.mock.calls.map((call) => call[0])).toEqual([
      '/bin/fish',
      'sh',
    ])
  })

  it('uses Windows-native lookup on win32', async () => {
    const runCommand = mock(async () => ({
      exitCode: 0,
      stdout: 'C:\\Users\\dev\\AppData\\Roaming\\npm\\codex.cmd\r\n',
      stderr: '',
    }))

    const result = await resolveHostBinary('codex', {
      env: { Path: 'C:\\Windows\\System32' },
      platform: 'win32',
      runCommand,
    })

    expect(result?.path).toBe(
      'C:\\Users\\dev\\AppData\\Roaming\\npm\\codex.cmd',
    )
    expect(runCommand).toHaveBeenCalledWith(
      'where.exe',
      ['codex'],
      expect.objectContaining({ env: { Path: 'C:\\Windows\\System32' } }),
    )
  })

  it('prepends the resolved binary directory to child PATH for shims', () => {
    expect(
      buildResolvedBinaryEnv({
        binaryPath: 'C:\\Users\\dev\\AppData\\Roaming\\npm\\codex.cmd',
        env: { Path: 'C:\\Windows\\System32' },
        platform: 'win32',
      }),
    ).toEqual({
      Path: 'C:\\Users\\dev\\AppData\\Roaming\\npm;C:\\Windows\\System32',
    })

    expect(
      buildResolvedBinaryEnv({
        binaryPath: '/Users/dev/.local/share/fnm/node/bin/codex',
        env: { PATH: '/usr/bin:/bin' },
        platform: 'darwin',
      }),
    ).toEqual({
      PATH: '/Users/dev/.local/share/fnm/node/bin:/usr/bin:/bin',
    })
  })
})
