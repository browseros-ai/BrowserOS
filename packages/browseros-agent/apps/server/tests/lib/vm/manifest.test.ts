/**
 * @license
 * Copyright 2025 BrowserOS
 */

import { afterEach, beforeEach, describe, expect, it } from 'bun:test'
import { mkdir, mkdtemp, readFile, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'
import { ManifestMissingError } from '../../../src/lib/vm/errors'
import {
  agentForArch,
  compareVersions,
  readCachedManifest,
  readInstalledManifest,
  type VmManifest,
  writeInstalledManifest,
} from '../../../src/lib/vm/manifest'

const manifest: VmManifest = {
  schemaVersion: 2,
  updatedAt: '2026-04-22T00:00:00.000Z',
  agents: {
    openclaw: {
      image: 'ghcr.io/openclaw/openclaw',
      version: '2026.4.12',
      tarballs: {
        arm64: {
          key: 'vm/images/openclaw-2026.4.12-arm64.tar.gz',
          sha256: 'c',
          sizeBytes: 3,
        },
        x64: {
          key: 'vm/images/openclaw-2026.4.12-x64.tar.gz',
          sha256: 'd',
          sizeBytes: 4,
        },
      },
    },
  },
}

describe('VM manifest helpers', () => {
  let root: string

  beforeEach(async () => {
    root = await mkdtemp(join(tmpdir(), 'browseros-vm-manifest-'))
  })

  afterEach(async () => {
    await rm(root, { recursive: true, force: true })
  })

  it('reads the cached manifest', async () => {
    const manifestPath = join(root, 'cache', 'vm', 'manifest.json')
    await mkdir(dirname(manifestPath), { recursive: true })
    await Bun.write(manifestPath, `${JSON.stringify(manifest)}\n`)

    await expect(readCachedManifest(root)).resolves.toEqual(manifest)
  })

  it('throws ManifestMissingError when cached manifest is absent', async () => {
    await expect(readCachedManifest(root)).rejects.toThrow(ManifestMissingError)
  })

  it('returns null for a missing installed manifest', async () => {
    await expect(readInstalledManifest(root)).resolves.toBeNull()
  })

  it('reads the installed manifest', async () => {
    const manifestPath = join(root, 'vm', 'manifest.json')
    await mkdir(dirname(manifestPath), { recursive: true })
    await Bun.write(manifestPath, `${JSON.stringify(manifest)}\n`)

    await expect(readInstalledManifest(root)).resolves.toEqual(manifest)
  })

  it('throws on malformed installed manifest JSON', async () => {
    const manifestPath = join(root, 'vm', 'manifest.json')
    await mkdir(dirname(manifestPath), { recursive: true })
    await Bun.write(manifestPath, '{not-json')

    await expect(readInstalledManifest(root)).rejects.toThrow()
  })

  it('writes the installed manifest atomically', async () => {
    await writeInstalledManifest(manifest, root)

    const raw = await readFile(join(root, 'vm', 'manifest.json'), 'utf8')
    expect(JSON.parse(raw)).toEqual(manifest)
  })

  it('compares installed and cached versions', () => {
    const older = { ...manifest, updatedAt: '2026-04-21T00:00:00.000Z' }
    const newer = { ...manifest, updatedAt: '2026-04-23T00:00:00.000Z' }

    expect(compareVersions(null, manifest)).toBe('fresh')
    expect(compareVersions(manifest, manifest)).toBe('same')
    expect(compareVersions(older, manifest)).toBe('upgrade')
    expect(compareVersions(newer, manifest)).toBe('downgrade')
  })

  it('compares ISO timestamp versions with time-of-day precision', () => {
    const morning = {
      ...manifest,
      updatedAt: '2026-04-22T10:00:00.000Z',
    }
    const afternoon = {
      ...manifest,
      updatedAt: '2026-04-22T15:00:00.000Z',
    }

    expect(compareVersions(morning, afternoon)).toBe('upgrade')
    expect(compareVersions(afternoon, morning)).toBe('downgrade')
  })

  it('returns the requested agent tarball for an arch', () => {
    expect(agentForArch(manifest, 'openclaw', 'arm64')).toEqual({
      image: 'ghcr.io/openclaw/openclaw',
      version: '2026.4.12',
      tarball: {
        key: 'vm/images/openclaw-2026.4.12-arm64.tar.gz',
        sha256: 'c',
        sizeBytes: 3,
      },
    })
  })

  it('throws when an agent or arch is absent', () => {
    expect(() => agentForArch(manifest, 'missing', 'arm64')).toThrow(
      'missing agent',
    )
    expect(() =>
      agentForArch(manifest, 'openclaw', 'x64' as never),
    ).not.toThrow()
  })
})
