/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */
import { afterEach, describe, expect, it } from 'bun:test'
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { loadOrCreateInstallationId } from '../../src/lib/installation-id'

describe('loadOrCreateInstallationId', () => {
  const tempDirs: string[] = []

  afterEach(async () => {
    await Promise.all(
      tempDirs.map((dir) => rm(dir, { recursive: true, force: true })),
    )
    tempDirs.length = 0
  })

  it('creates the one-field installation file and reuses its UUID', async () => {
    const root = await makeTempDir()

    const first = await loadOrCreateInstallationId(root)
    const second = await loadOrCreateInstallationId(root)

    expect(second).toBe(first)
    expect(
      JSON.parse(await readFile(join(root, 'installation.json'), 'utf8')),
    ).toEqual({ install_id: first })
  })

  it('makes concurrent creators converge on one UUID', async () => {
    const root = await makeTempDir()

    const ids = await Promise.all(
      Array.from({ length: 12 }, () => loadOrCreateInstallationId(root)),
    )

    expect(new Set(ids).size).toBe(1)
  })

  it('preserves a malformed existing file instead of rotating identity', async () => {
    const root = await makeTempDir()
    const installationPath = join(root, 'installation.json')
    await writeFile(installationPath, '{not json', 'utf8')

    await expect(loadOrCreateInstallationId(root)).rejects.toThrow(
      'Invalid installation identity file',
    )
    expect(await readFile(installationPath, 'utf8')).toBe('{not json')
  })

  async function makeTempDir(): Promise<string> {
    const dir = await mkdtemp(join(tmpdir(), 'browseros-installation-test-'))
    tempDirs.push(dir)
    return dir
  }
})
