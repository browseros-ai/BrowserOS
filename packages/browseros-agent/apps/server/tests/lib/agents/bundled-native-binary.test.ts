/**
 * @license
 * Copyright 2025 BrowserOS
 */

import { afterEach, describe, expect, it } from 'bun:test'
import { chmod, mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'
import {
  resolveBundledNativeBinary,
  withBundledNativeBinaryPath,
} from '../../../src/lib/agents/host-acp/bundled-native-binary'

describe('bundled native binary helpers', () => {
  const tempDirs: string[] = []

  afterEach(async () => {
    await Promise.all(
      tempDirs.map((dir) => rm(dir, { recursive: true, force: true })),
    )
    tempDirs.length = 0
  })

  it('resolves executable Unix bundled binaries and prepends their directory', async () => {
    const resourcesDir = await mkdtemp(join(tmpdir(), 'browseros-native-cli-'))
    tempDirs.push(resourcesDir)
    const codexPath = join(resourcesDir, 'bin', 'third_party', 'codex')
    await mkdir(dirname(codexPath), { recursive: true })
    await writeFile(codexPath, '#!/bin/sh\n')
    await chmod(codexPath, 0o755)

    expect(
      resolveBundledNativeBinary({
        adapter: 'codex',
        resourcesDir,
        env: { PATH: '/usr/bin' },
        platform: 'linux',
      }),
    ).toEqual({
      path: codexPath,
      env: {
        PATH: `${dirname(codexPath)}:/usr/bin`,
      },
    })
  })

  it('ignores non-executable Unix bundled binaries', async () => {
    const resourcesDir = await mkdtemp(join(tmpdir(), 'browseros-native-cli-'))
    tempDirs.push(resourcesDir)
    const claudePath = join(resourcesDir, 'bin', 'third_party', 'claude')
    await mkdir(dirname(claudePath), { recursive: true })
    await writeFile(claudePath, '#!/bin/sh\n')
    await chmod(claudePath, 0o644)

    expect(
      resolveBundledNativeBinary({
        adapter: 'claude',
        resourcesDir,
        platform: 'darwin',
      }),
    ).toBeNull()
  })

  it('resolves Windows bundled binaries without executable bits', async () => {
    const resourcesDir = await mkdtemp(join(tmpdir(), 'browseros-native-cli-'))
    tempDirs.push(resourcesDir)
    const claudePath = join(resourcesDir, 'bin', 'third_party', 'claude.exe')
    await mkdir(dirname(claudePath), { recursive: true })
    await writeFile(claudePath, 'MZ')

    expect(
      resolveBundledNativeBinary({
        adapter: 'claude',
        resourcesDir,
        env: { Path: 'C:\\Windows\\System32' },
        platform: 'win32',
      }),
    ).toEqual({
      path: claudePath,
      env: {
        Path: `${dirname(claudePath)};C:\\Windows\\System32`,
      },
    })
  })

  it('prepends the bundled CLI directory once for ACP adapter commands', async () => {
    const resourcesDir = await mkdtemp(join(tmpdir(), 'browseros-native-cli-'))
    tempDirs.push(resourcesDir)
    const bundledDir = join(resourcesDir, 'bin', 'third_party')
    await mkdir(bundledDir, { recursive: true })

    expect(
      withBundledNativeBinaryPath({
        resourcesDir,
        env: { PATH: `/opt/bin:${bundledDir}` },
        platform: 'linux',
      }),
    ).toEqual({
      PATH: `${bundledDir}:/opt/bin`,
    })

    expect(
      withBundledNativeBinaryPath({
        resourcesDir,
        env: { PATH: '/opt/bin' },
        platform: 'linux',
      }),
    ).toEqual({
      PATH: `${bundledDir}:/opt/bin`,
    })
  })
})
