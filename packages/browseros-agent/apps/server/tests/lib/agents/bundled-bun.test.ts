/**
 * @license
 * Copyright 2025 BrowserOS
 */

import { afterEach, describe, expect, it } from 'bun:test'
import {
  chmod,
  mkdir,
  mkdtemp,
  readFile,
  rm,
  stat,
  writeFile,
} from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'
import {
  resolveBundledBun,
  withBundledBunAcpAdapterEnv,
} from '../../../src/lib/agents/host-acp/bundled-bun'

describe('bundled Bun helpers', () => {
  const tempDirs: string[] = []

  afterEach(async () => {
    await Promise.all(
      tempDirs.map((dir) => rm(dir, { recursive: true, force: true })),
    )
    tempDirs.length = 0
  })

  it('resolves the macOS bundled Bun executable', async () => {
    const resourcesDir = await mkdtemp(join(tmpdir(), 'browseros-bun-'))
    tempDirs.push(resourcesDir)
    const bunPath = join(resourcesDir, 'bin', 'third_party', 'bun')
    await mkdir(dirname(bunPath), { recursive: true })
    await writeFile(bunPath, '#!/bin/sh\n')
    await chmod(bunPath, 0o755)

    expect(resolveBundledBun({ resourcesDir, platform: 'darwin' })).toBe(
      bunPath,
    )
  })

  it('resolves the Linux bundled Bun executable', async () => {
    const resourcesDir = await mkdtemp(join(tmpdir(), 'browseros-bun-'))
    tempDirs.push(resourcesDir)
    const bunPath = join(resourcesDir, 'bin', 'third_party', 'bun')
    await mkdir(dirname(bunPath), { recursive: true })
    await writeFile(bunPath, '#!/bin/sh\n')
    await chmod(bunPath, 0o755)

    expect(resolveBundledBun({ resourcesDir, platform: 'linux' })).toBe(bunPath)
  })

  it('resolves the Windows bundled Bun executable', async () => {
    const resourcesDir = await mkdtemp(join(tmpdir(), 'browseros-bun-'))
    tempDirs.push(resourcesDir)
    const bunPath = join(resourcesDir, 'bin', 'third_party', 'bun.exe')
    await mkdir(dirname(bunPath), { recursive: true })
    await writeFile(bunPath, 'MZ')

    expect(resolveBundledBun({ resourcesDir, platform: 'win32' })).toBe(bunPath)
  })

  it('ignores non-executable bundled Bun files on macOS', async () => {
    const resourcesDir = await mkdtemp(join(tmpdir(), 'browseros-bun-'))
    tempDirs.push(resourcesDir)
    const bunPath = join(resourcesDir, 'bin', 'third_party', 'bun')
    await mkdir(dirname(bunPath), { recursive: true })
    await writeFile(bunPath, '#!/bin/sh\n')
    await chmod(bunPath, 0o644)

    expect(resolveBundledBun({ resourcesDir, platform: 'darwin' })).toBeNull()
  })

  it('ignores non-executable bundled Bun files on Linux', async () => {
    const resourcesDir = await mkdtemp(join(tmpdir(), 'browseros-bun-'))
    tempDirs.push(resourcesDir)
    const bunPath = join(resourcesDir, 'bin', 'third_party', 'bun')
    await mkdir(dirname(bunPath), { recursive: true })
    await writeFile(bunPath, '#!/bin/sh\n')
    await chmod(bunPath, 0o644)

    expect(resolveBundledBun({ resourcesDir, platform: 'linux' })).toBeNull()
  })

  it('ignores bundled Bun on unsupported platforms', async () => {
    const resourcesDir = await mkdtemp(join(tmpdir(), 'browseros-bun-'))
    tempDirs.push(resourcesDir)
    const bunPath = join(resourcesDir, 'bin', 'third_party', 'bun')
    await mkdir(dirname(bunPath), { recursive: true })
    await writeFile(bunPath, '#!/bin/sh\n')

    expect(resolveBundledBun({ resourcesDir, platform: 'freebsd' })).toBeNull()
  })

  it('creates a private node shim for Unix ACP adapter launches', async () => {
    const resourcesDir = await mkdtemp(join(tmpdir(), 'browseros-bun-'))
    const browserosDir = await mkdtemp(join(tmpdir(), 'browseros-dir-'))
    tempDirs.push(resourcesDir, browserosDir)
    const bunPath = join(resourcesDir, 'bin', 'third_party', 'bun')
    await mkdir(dirname(bunPath), { recursive: true })
    await writeFile(bunPath, '#!/bin/sh\n')
    await chmod(bunPath, 0o755)

    const env = withBundledBunAcpAdapterEnv({
      bunPath,
      browserosDir,
      env: { PATH: '/usr/bin' },
      platform: 'darwin',
    })

    const shimPath = join(browserosDir, 'cache', 'acp-node-shim', 'node')
    expect(env.PATH.split(':').slice(0, 3)).toEqual([
      dirname(shimPath),
      dirname(bunPath),
      '/usr/bin',
    ])
    expect(env.BUN_INSTALL_CACHE_DIR).toBe(
      join(browserosDir, 'cache', 'bun-install'),
    )
    expect(await readFile(shimPath, 'utf8')).toContain(bunPath)
    expect((await stat(shimPath)).mode & 0o111).not.toBe(0)
  })

  it('prepends the bundled Bun directory even without a writable BrowserOS dir', async () => {
    const resourcesDir = await mkdtemp(join(tmpdir(), 'browseros-bun-'))
    tempDirs.push(resourcesDir)
    const bunPath = join(resourcesDir, 'bin', 'third_party', 'bun')
    await mkdir(dirname(bunPath), { recursive: true })
    await writeFile(bunPath, '#!/bin/sh\n')
    await chmod(bunPath, 0o755)

    const env = withBundledBunAcpAdapterEnv({
      bunPath,
      env: { PATH: '/usr/bin' },
      platform: 'linux',
    })

    expect(env.PATH.split(':').slice(0, 2)).toEqual([
      dirname(bunPath),
      '/usr/bin',
    ])
    expect(env.BUN_INSTALL_CACHE_DIR).toBeUndefined()
  })
})
