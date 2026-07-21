/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { afterEach, beforeEach, describe, expect, test } from 'bun:test'
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { env } from '../../src/env'
import { writeRuntimeFile } from '../../src/lib/runtime-file'

const prior = {
  browserClawDirOverride: env.browserClawDirOverride,
}

let tmp: string

beforeEach(() => {
  tmp = mkdtempSync(join(tmpdir(), 'claw-runtime-'))
  env.browserClawDirOverride = tmp
})

afterEach(() => {
  env.browserClawDirOverride = prior.browserClawDirOverride
  rmSync(tmp, { recursive: true, force: true })
})

describe('writeRuntimeFile', () => {
  test('writes a JSON blob with the running URL', async () => {
    await writeRuntimeFile('http://127.0.0.1:9200')

    const raw = readFileSync(join(tmp, 'runtime.json'), 'utf8')
    expect(JSON.parse(raw)).toEqual({ url: 'http://127.0.0.1:9200' })
  })

  test('overwrites an existing runtime file with the new URL', async () => {
    writeFileSync(
      join(tmp, 'runtime.json'),
      JSON.stringify({ url: 'http://127.0.0.1:9500' }),
      'utf8',
    )

    await writeRuntimeFile('http://127.0.0.1:9600')

    const raw = readFileSync(join(tmp, 'runtime.json'), 'utf8')
    expect(JSON.parse(raw)).toEqual({ url: 'http://127.0.0.1:9600' })
  })

  test('creates the state directory if it does not exist', async () => {
    // Point the override at a not-yet-created subdir.
    env.browserClawDirOverride = join(tmp, 'nested', 'state')

    await writeRuntimeFile('http://127.0.0.1:9700')

    const raw = readFileSync(
      join(tmp, 'nested', 'state', 'runtime.json'),
      'utf8',
    )
    expect(JSON.parse(raw)).toEqual({ url: 'http://127.0.0.1:9700' })
  })

  test('does not leave the .tmp sidecar around after a successful write', async () => {
    await writeRuntimeFile('http://127.0.0.1:9200')

    // After atomic rename, only runtime.json survives.
    expect(() => readFileSync(join(tmp, 'runtime.json'), 'utf8')).not.toThrow()
    expect(() => readFileSync(join(tmp, 'runtime.json.tmp'), 'utf8')).toThrow()
  })
})
