/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { describe, expect, test } from 'bun:test'
import { existsSync } from 'node:fs'
import { readdir, writeFile } from 'node:fs/promises'
import { join } from 'node:path'
import { z } from 'zod'
import {
  ensureDir,
  fileExists,
  listFiles,
  readJson,
  removeFile,
  StorageCorruptError,
  StorageInvalidPathError,
  StorageNotFoundError,
  writeJson,
} from '../../src/lib/storage'
import { withTempBrowserosDir } from '../_helpers/temp-browseros-dir'

const sampleSchema = z.object({
  name: z.string(),
  ok: z.boolean(),
})

describe('storage', () => {
  test('writeJson then readJson round-trips through the schema', async () => {
    await withTempBrowserosDir(async () => {
      await writeJson('sample.json', { name: 'one', ok: true }, sampleSchema)
      const read = await readJson('sample.json', sampleSchema)
      expect(read).toEqual({ name: 'one', ok: true })
    })
  })

  test('writeJson creates parent directories', async () => {
    await withTempBrowserosDir(async (dir) => {
      await writeJson(
        'nested/dir/sample.json',
        { name: 'nested', ok: true },
        sampleSchema,
      )
      expect(existsSync(join(dir, 'claw-server/nested/dir/sample.json'))).toBe(
        true,
      )
    })
  })

  test('writeJson is atomic: the .tmp file is renamed, not left behind', async () => {
    await withTempBrowserosDir(async (dir) => {
      await writeJson('atomic.json', { name: 'a', ok: true }, sampleSchema)
      const entries = await readdir(join(dir, 'claw-server'))
      expect(entries).toContain('atomic.json')
      expect(entries.some((entry) => entry.endsWith('.tmp'))).toBe(false)
    })
  })

  test('readJson throws StorageNotFoundError on missing file', async () => {
    await withTempBrowserosDir(async () => {
      expect(readJson('ghost.json', sampleSchema)).rejects.toBeInstanceOf(
        StorageNotFoundError,
      )
    })
  })

  test('readJson throws StorageCorruptError when JSON is invalid', async () => {
    await withTempBrowserosDir(async (dir) => {
      const root = join(dir, 'claw-server')
      await ensureDir('.')
      await writeFile(join(root, 'broken.json'), '{not-json', 'utf8')
      expect(readJson('broken.json', sampleSchema)).rejects.toBeInstanceOf(
        StorageCorruptError,
      )
    })
  })

  test('readJson throws StorageCorruptError when schema rejects the value', async () => {
    await withTempBrowserosDir(async (dir) => {
      const root = join(dir, 'claw-server')
      await ensureDir('.')
      await writeFile(
        join(root, 'wrong-shape.json'),
        JSON.stringify({ name: 1, ok: 'no' }),
        'utf8',
      )
      expect(readJson('wrong-shape.json', sampleSchema)).rejects.toBeInstanceOf(
        StorageCorruptError,
      )
    })
  })

  test('writeJson refuses values that do not satisfy the schema', async () => {
    await withTempBrowserosDir(async () => {
      // biome-ignore lint/suspicious/noExplicitAny: deliberate invalid input for the test
      const bad = { name: 5, ok: 'not a bool' } as any
      expect(
        writeJson('rejected.json', bad, sampleSchema),
      ).rejects.toBeInstanceOf(StorageCorruptError)
    })
  })

  test('removeFile deletes an existing file and returns true', async () => {
    await withTempBrowserosDir(async () => {
      await writeJson('to-delete.json', { name: 'd', ok: true }, sampleSchema)
      expect(await removeFile('to-delete.json')).toBe(true)
      expect(await fileExists('to-delete.json')).toBe(false)
    })
  })

  test('removeFile returns false when the file does not exist', async () => {
    await withTempBrowserosDir(async () => {
      expect(await removeFile('never-existed.json')).toBe(false)
    })
  })

  test('listFiles defaults to .json and filters non-matching entries', async () => {
    await withTempBrowserosDir(async (dir) => {
      const root = join(dir, 'claw-server', 'list-test')
      await ensureDir('list-test')
      await writeFile(join(root, 'a.json'), '{"name":"a","ok":true}', 'utf8')
      await writeFile(join(root, 'b.json'), '{"name":"b","ok":true}', 'utf8')
      await writeFile(join(root, 'c.txt'), 'unrelated', 'utf8')
      const names = await listFiles('list-test')
      expect(names.sort()).toEqual(['a.json', 'b.json'])
    })
  })

  test('listFiles returns [] when the directory does not exist', async () => {
    await withTempBrowserosDir(async () => {
      expect(await listFiles('missing-dir')).toEqual([])
    })
  })

  test('relative paths cannot escape the claw-server root', async () => {
    await withTempBrowserosDir(async () => {
      expect(() =>
        writeJson('../escape.json', { name: 'e', ok: true }, sampleSchema),
      ).toThrow(StorageInvalidPathError)
      expect(() => readJson('../escape.json', sampleSchema)).toThrow(
        StorageInvalidPathError,
      )
    })
  })

  test('absolute paths are rejected', async () => {
    await withTempBrowserosDir(async () => {
      expect(() => readJson('/etc/passwd', sampleSchema)).toThrow(
        StorageInvalidPathError,
      )
    })
  })

  test('lateral traversal that stays inside the claw-server root is still rejected', async () => {
    await withTempBrowserosDir(async () => {
      // `agents/../config.json` normalises to `config.json` which sits
      // INSIDE the claw-server root but escapes the intended subdirectory.
      // The guard must catch the raw `..` before normalize collapses it.
      expect(() => readJson('agents/../config.json', sampleSchema)).toThrow(
        StorageInvalidPathError,
      )
      expect(() =>
        writeJson(
          'agents/../config.json',
          { name: 'x', ok: true },
          sampleSchema,
        ),
      ).toThrow(StorageInvalidPathError)
      expect(() => removeFile('agents/../config.json')).toThrow(
        StorageInvalidPathError,
      )
    })
  })
})
