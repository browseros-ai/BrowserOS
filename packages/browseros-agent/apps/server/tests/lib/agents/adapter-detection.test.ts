/**
 * @license
 * Copyright 2025 BrowserOS
 */

import { afterEach, describe, expect, it } from 'bun:test'
import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import {
  detectHostAdapter,
  probeNpxPackageCache,
} from '../../../src/lib/agents/host-acp/detection'

describe('adapter detection', () => {
  it('reports ready when the adapter package is cached and auth probe succeeds', async () => {
    const result = await detectHostAdapter('claude', {
      now: () => 1234,
      resolveBinary: async (name) => {
        if (name === 'claude') {
          return {
            path: '/Users/dev/.local/bin/claude',
            env: { PATH: '/Users/dev/.local/bin:/usr/bin' },
          }
        }
        if (name === 'npx') {
          return {
            path: '/opt/node/bin/npx',
            env: { PATH: '/opt/node/bin:/usr/bin' },
          }
        }
        return null
      },
      runCommand: async (cmd, args) => {
        if (cmd === '/Users/dev/.local/bin/claude' && args[0] === '--version') {
          return { exitCode: 0, stdout: 'claude 1.2.3\n', stderr: '' }
        }
        if (
          cmd === '/Users/dev/.local/bin/claude' &&
          args.join(' ') === 'auth status'
        ) {
          return { exitCode: 0, stdout: 'authenticated\n', stderr: '' }
        }
        throw new Error(`unexpected command ${cmd} ${args.join(' ')}`)
      },
      probePackageCache: async (packageName, versionRange) => {
        expect(packageName).toBe('@agentclientprotocol/claude-agent-acp')
        expect(versionRange).toBe('^0.31.0')
        return true
      },
    })

    expect(result).toMatchObject({
      healthy: true,
      readiness: 'ready',
      installState: 'installed',
      nativeCliState: 'present',
      authState: 'authenticated',
      adapterLaunchSource: 'host-npx',
      packageCacheState: 'cached',
      version: 'claude 1.2.3',
      checkedAt: 1234,
    })
  })

  it('surfaces auth blockers separately from install state', async () => {
    const result = await detectHostAdapter('claude', {
      now: () => 1234,
      resolveBinary: async (name) =>
        name === 'claude'
          ? { path: '/bin/claude', env: { PATH: '/bin' } }
          : { path: '/bin/npx', env: { PATH: '/bin' } },
      runCommand: async (_cmd, args) =>
        args[0] === '--version'
          ? { exitCode: 0, stdout: 'claude 1.2.3\n', stderr: '' }
          : { exitCode: 1, stdout: '', stderr: 'not logged in' },
      probePackageCache: async () => true,
    })

    expect(result).toMatchObject({
      healthy: false,
      readiness: 'needs-auth',
      installState: 'installed',
      nativeCliState: 'present',
      authState: 'unauthenticated',
      reason: 'Claude Code is installed but is not authenticated.',
    })
  })

  it('reports fetchable npx packages without pretending the native CLI is present', async () => {
    const result = await detectHostAdapter('codex', {
      now: () => 1234,
      resolveBinary: async (name) =>
        name === 'npx' ? { path: '/bin/npx', env: { PATH: '/bin' } } : null,
      runCommand: async () => {
        throw new Error('should not run native probes without a native CLI')
      },
      probePackageCache: async () => false,
    })

    expect(result).toMatchObject({
      healthy: false,
      readiness: 'will-fetch-package',
      installState: 'npx-available',
      nativeCliState: 'missing',
      authState: 'unknown',
      adapterLaunchSource: 'host-npx',
      packageCacheState: 'fetch-required',
    })
  })

  it('warns when the adapter package is cached but native auth cannot be verified', async () => {
    const result = await detectHostAdapter('codex', {
      now: () => 1234,
      resolveBinary: async (name) =>
        name === 'npx' ? { path: '/bin/npx', env: { PATH: '/bin' } } : null,
      runCommand: async () => {
        throw new Error('should not run native probes without a native CLI')
      },
      probePackageCache: async () => true,
    })

    expect(result).toMatchObject({
      healthy: true,
      readiness: 'diagnostic-warning',
      installState: 'installed',
      nativeCliState: 'missing',
      authState: 'unknown',
      adapterLaunchSource: 'host-npx',
      packageCacheState: 'cached',
      reason: 'Codex can launch, but authentication could not be verified.',
    })
  })

  it('detects bundled Bun as a package runner without host npx', async () => {
    const result = await detectHostAdapter('codex', {
      now: () => 1234,
      platform: 'darwin',
      resourcesDir: '/Applications/BrowserOS.app/Contents/Resources',
      resolveBundledBun: () =>
        '/Applications/BrowserOS.app/Contents/Resources/bin/third_party/bun',
      resolveBinary: async () => null,
      runCommand: async () => {
        throw new Error('should not run native probes without a native CLI')
      },
      probePackageCache: async () => false,
    })

    expect(result).toMatchObject({
      healthy: true,
      readiness: 'diagnostic-warning',
      installState: 'package-runner-available',
      nativeCliState: 'missing',
      authState: 'unknown',
      adapterLaunchSource: 'bundled-bun',
      packageCacheState: 'unknown',
      reason: 'Codex can launch, but authentication could not be verified.',
    })
  })

  it('uses bundled native CLIs before host PATH for version and auth probes', async () => {
    const resourcesDir = '/Applications/BrowserOS.app/Contents/Resources'
    const bundledDir = join(resourcesDir, 'bin', 'third_party')
    const bundledCodex = join(bundledDir, 'codex')
    const hostResolveCalls: string[] = []
    const commandCalls: Array<{
      cmd: string
      args: string[]
      pathEnv: string | undefined
    }> = []

    const result = await detectHostAdapter('codex', {
      now: () => 1234,
      platform: 'darwin',
      resourcesDir,
      resolveBundledBun: () => join(bundledDir, 'bun'),
      resolveBundledNativeBinary: ({ adapter, env, platform }) => {
        expect(adapter).toBe('codex')
        expect(platform).toBe('darwin')
        expect(env?.PATH).toBe('/usr/bin')
        return {
          path: bundledCodex,
          env: { PATH: `${bundledDir}:/usr/bin` },
        }
      },
      env: { PATH: '/usr/bin' },
      resolveBinary: async (name) => {
        hostResolveCalls.push(name)
        return null
      },
      runCommand: async (cmd, args, options) => {
        commandCalls.push({ cmd, args, pathEnv: options.env?.PATH })
        if (args[0] === '--version') {
          return { exitCode: 0, stdout: 'codex-cli 0.136.0\n', stderr: '' }
        }
        if (args.join(' ') === 'login status') {
          return { exitCode: 0, stdout: 'authenticated\n', stderr: '' }
        }
        throw new Error(`unexpected command ${cmd} ${args.join(' ')}`)
      },
      probePackageCache: async () => false,
    })

    expect(result).toMatchObject({
      healthy: true,
      readiness: 'ready',
      installState: 'installed',
      nativeCliState: 'present',
      authState: 'authenticated',
      adapterLaunchSource: 'bundled-bun',
      packageCacheState: 'unknown',
      version: 'codex-cli 0.136.0',
      checkedAt: 1234,
    })
    expect(hostResolveCalls).toEqual([])
    expect(commandCalls).toEqual([
      {
        cmd: bundledCodex,
        args: ['--version'],
        pathEnv: `${bundledDir}:/usr/bin`,
      },
      {
        cmd: bundledCodex,
        args: ['login', 'status'],
        pathEnv: `${bundledDir}:/usr/bin`,
      },
    ])
  })

  it('reports missing package runner separately from bundled native CLI presence', async () => {
    const resourcesDir = '/opt/BrowserOS/resources'
    const bundledDir = join(resourcesDir, 'bin', 'third_party')
    const bundledCodex = join(bundledDir, 'codex')

    const result = await detectHostAdapter('codex', {
      now: () => 1234,
      platform: 'linux',
      resourcesDir,
      resolveBundledNativeBinary: () => ({
        path: bundledCodex,
        env: { PATH: `${bundledDir}:/usr/bin` },
      }),
      resolveBundledBun: () => null,
      resolveBinary: async () => null,
      runCommand: async (_cmd, args) => {
        if (args[0] === '--version') {
          return { exitCode: 0, stdout: 'codex-cli 0.136.0\n', stderr: '' }
        }
        if (args.join(' ') === 'login status') {
          return { exitCode: 0, stdout: 'authenticated\n', stderr: '' }
        }
        throw new Error(`unexpected command ${args.join(' ')}`)
      },
      probePackageCache: async () => false,
    })

    expect(result).toMatchObject({
      healthy: false,
      readiness: 'needs-install',
      installState: 'installed',
      nativeCliState: 'present',
      authState: 'authenticated',
      adapterLaunchSource: 'none',
      packageCacheState: 'unknown',
      reason:
        'Codex adapter package cannot launch because neither bundled Bun nor npx is available.',
    })
  })

  it('uses codex doctor when login status is not supported', async () => {
    const calls: Array<{ args: string[]; timeoutMs: number | undefined }> = []

    const result = await detectHostAdapter('codex', {
      now: () => 1234,
      timeoutMs: 3000,
      resolveBinary: async (name) => {
        if (name === 'codex') {
          return { path: '/bin/codex', env: { PATH: '/bin' } }
        }
        if (name === 'npx') {
          return { path: '/bin/npx', env: { PATH: '/bin' } }
        }
        return null
      },
      runCommand: async (_cmd, args, options) => {
        calls.push({ args, timeoutMs: options.timeoutMs })
        if (args[0] === '--version') {
          return { exitCode: 0, stdout: 'codex-cli 0.135.0\n', stderr: '' }
        }
        if (args.join(' ') === 'login status') {
          return { exitCode: 2, stdout: '', stderr: 'unrecognized command' }
        }
        if (args[0] === 'doctor') {
          return { exitCode: 0, stdout: 'Auth: authenticated\n', stderr: '' }
        }
        throw new Error(`unexpected command ${args.join(' ')}`)
      },
      probePackageCache: async () => true,
    })

    expect(result).toMatchObject({
      healthy: true,
      readiness: 'ready',
      authState: 'authenticated',
      version: 'codex-cli 0.135.0',
    })
    expect(calls).toContainEqual({ args: ['login', 'status'], timeoutMs: 1500 })
    expect(calls).toContainEqual({ args: ['doctor'], timeoutMs: 1500 })
  })

  it('warns when native auth probing fails', async () => {
    const result = await detectHostAdapter('claude', {
      now: () => 1234,
      resolveBinary: async (name) =>
        name === 'claude'
          ? { path: '/bin/claude', env: { PATH: '/bin' } }
          : { path: '/bin/npx', env: { PATH: '/bin' } },
      runCommand: async (_cmd, args) => {
        if (args[0] === '--version') {
          return { exitCode: 0, stdout: 'claude 1.2.3\n', stderr: '' }
        }
        throw new Error('auth probe failed')
      },
      probePackageCache: async () => true,
    })

    expect(result).toMatchObject({
      healthy: true,
      readiness: 'diagnostic-warning',
      nativeCliState: 'present',
      authState: 'unknown',
      reason:
        'Claude Code can launch, but authentication could not be verified.',
    })
  })
})

describe('probeNpxPackageCache', () => {
  const tempDirs: string[] = []

  afterEach(async () => {
    await Promise.all(
      tempDirs.map((dir) => rm(dir, { recursive: true, force: true })),
    )
    tempDirs.length = 0
  })

  it('finds cached scoped npx packages that match the requested range', async () => {
    const npmCacheDir = await mkdtemp(join(tmpdir(), 'browseros-npx-cache-'))
    tempDirs.push(npmCacheDir)
    await mkdir(
      join(
        npmCacheDir,
        '_npx',
        'hit',
        'node_modules',
        '@agentclientprotocol',
        'codex-acp',
      ),
      { recursive: true },
    )
    await mkdir(
      join(
        npmCacheDir,
        '_npx',
        'miss',
        'node_modules',
        '@agentclientprotocol',
        'nested',
        'node_modules',
        '@agentclientprotocol',
        'codex-acp',
      ),
      { recursive: true },
    )
    await writeFile(
      join(
        npmCacheDir,
        '_npx',
        'hit',
        'node_modules',
        '@agentclientprotocol',
        'codex-acp',
        'package.json',
      ),
      '{"version":"1.0.2"}',
    )
    await writeFile(
      join(
        npmCacheDir,
        '_npx',
        'miss',
        'node_modules',
        '@agentclientprotocol',
        'nested',
        'node_modules',
        '@agentclientprotocol',
        'codex-acp',
        'package.json',
      ),
      '{"version":"1.0.2"}',
    )

    await expect(
      probeNpxPackageCache('@agentclientprotocol/codex-acp', {
        npxCacheDir: join(npmCacheDir, '_npx'),
        versionRange: '^1.0.2',
      }),
    ).resolves.toBe(true)
  })

  it('ignores cached npx packages outside the requested range', async () => {
    const npmCacheDir = await mkdtemp(join(tmpdir(), 'browseros-npx-cache-'))
    tempDirs.push(npmCacheDir)
    await mkdir(
      join(
        npmCacheDir,
        '_npx',
        'stale',
        'node_modules',
        '@agentclientprotocol',
        'codex-acp',
      ),
      { recursive: true },
    )
    await writeFile(
      join(
        npmCacheDir,
        '_npx',
        'stale',
        'node_modules',
        '@agentclientprotocol',
        'codex-acp',
        'package.json',
      ),
      '{"version":"1.0.1"}',
    )

    await expect(
      probeNpxPackageCache('@agentclientprotocol/codex-acp', {
        npxCacheDir: join(npmCacheDir, '_npx'),
        versionRange: '^1.0.2',
      }),
    ).resolves.toBe(false)
  })
})
