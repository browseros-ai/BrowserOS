/**
 * @license
 * Copyright 2025 BrowserOS
 */

import { afterEach, describe, expect, it } from 'bun:test'
import type { Stats } from 'node:fs'
import { mkdtemp, rm, stat, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { ensureDirectory } from '../../src/lib/ensure-directory'

describe('ensureDirectory', () => {
  const tempDirs: string[] = []

  afterEach(async () => {
    await Promise.all(
      tempDirs.map((dir) => rm(dir, { recursive: true, force: true })),
    )
    tempDirs.length = 0
  })

  it('creates missing nested directories', async () => {
    const root = await mkdtemp(join(tmpdir(), 'browseros-ensure-dir-'))
    tempDirs.push(root)
    const target = join(root, 'OneDrive', 'South Hills OS')

    await ensureDirectory(target)

    expect((await stat(target)).isDirectory()).toBe(true)
  })

  it('treats EEXIST as success when the requested directory exists', async () => {
    const target = 'C:\\Users\\user\\OneDrive\\South Hills OS'
    const eexist = Object.assign(
      new Error(
        "EEXIST: file already exists, mkdir 'C:\\Users\\user\\OneDrive'",
      ),
      { code: 'EEXIST', path: 'C:\\Users\\user\\OneDrive' },
    )
    let statPath: string | undefined

    await ensureDirectory(target, {
      mkdir: (async () => {
        throw eexist
      }) as typeof import('node:fs/promises').mkdir,
      stat: (async (path: string) => {
        statPath = path
        return {
          isDirectory: () => true,
        } as Stats
      }) as typeof import('node:fs/promises').stat,
    })

    expect(statPath).toBe(target)
  })

  it('does not hide EEXIST when the requested path is not a directory', async () => {
    const root = await mkdtemp(join(tmpdir(), 'browseros-ensure-dir-'))
    tempDirs.push(root)
    const target = join(root, 'not-a-dir')
    await writeFile(target, 'file')

    await expect(ensureDirectory(target)).rejects.toThrow(/EEXIST|ENOTDIR/)
  })
})
