/**
 * @license
 * Copyright 2025 BrowserOS
 */

import { beforeEach, describe, it } from 'bun:test'
import assert from 'node:assert'
import { existsSync, mkdtempSync } from 'node:fs'
import { mkdir, readFile, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'

import { createMemoryRoutes } from '../../../src/api/routes/memory'
import { createSoulRoutes } from '../../../src/api/routes/soul'
import {
  getCoreMemoryPath,
  getMemoryDir,
  getSoulPath,
} from '../../../src/lib/browseros-dir'

describe('memory and soul reset routes', () => {
  beforeEach(() => {
    process.env.BROWSEROS_DIR = mkdtempSync(
      join(tmpdir(), 'browseros-reset-routes-'),
    )
  })

  it('deletes all memory files and leaves the memory directory usable', async () => {
    await mkdir(getMemoryDir(), { recursive: true })
    await writeFile(getCoreMemoryPath(), 'core facts')
    await writeFile(join(getMemoryDir(), '2026-05-09.md'), 'daily notes')

    const route = createMemoryRoutes()
    const response = await route.request('/', { method: 'DELETE' })

    assert.strictEqual(response.status, 200)
    assert.deepStrictEqual(await response.json(), { success: true })
    assert.strictEqual(existsSync(getMemoryDir()), true)
    assert.strictEqual(existsSync(getCoreMemoryPath()), false)
    assert.strictEqual(existsSync(join(getMemoryDir(), '2026-05-09.md')), false)

    const getResponse = await route.request('/')
    assert.deepStrictEqual(await getResponse.json(), { content: '' })
  })

  it('resets SOUL.md to the default template', async () => {
    await mkdir(dirname(getSoulPath()), { recursive: true })
    await writeFile(getSoulPath(), '# Custom soul\nBe different.')

    const route = createSoulRoutes()
    const response = await route.request('/', { method: 'DELETE' })

    assert.strictEqual(response.status, 200)
    const body = await response.json()
    assert.strictEqual(body.truncated, false)
    assert.ok(body.linesWritten > 0)

    const content = await readFile(getSoulPath(), 'utf8')
    assert.ok(
      content.includes("You're not a chatbot. You're becoming someone."),
    )
    assert.ok(!content.includes('Custom soul'))
  })
})
