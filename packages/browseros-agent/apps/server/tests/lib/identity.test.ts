/**
 * @license
 * Copyright 2025 BrowserOS
 */

import { afterEach, describe, expect, it } from 'bun:test'
import { mkdtempSync } from 'node:fs'
import { readFile, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { IdentityService } from '../../src/lib/identity'

describe('IdentityService', () => {
  const tempDirs: string[] = []

  afterEach(async () => {
    await Promise.all(
      tempDirs.map((dir) => rm(dir, { recursive: true, force: true })),
    )
    tempDirs.length = 0
  })

  it('uses the install id when config provides one', () => {
    const service = new IdentityService()

    service.initialize({ installId: 'install-123' })

    expect(service.getBrowserOSId()).toBe('install-123')
  })

  it('ignores an empty install id and generates a fallback id', () => {
    const dir = mkTempDir()
    const statePath = join(dir, 'identity', 'browseros-id.json')
    const service = new IdentityService()

    service.initialize({ installId: '', statePath })

    expect(service.getBrowserOSId()).not.toBe('')
  })

  it('persists a generated fallback id without using the database', async () => {
    const dir = mkTempDir()
    const statePath = join(dir, 'identity', 'browseros-id.json')

    const first = new IdentityService()
    first.initialize({ statePath })
    const id = first.getBrowserOSId()

    const second = new IdentityService()
    second.initialize({ statePath })

    expect(second.getBrowserOSId()).toBe(id)
    expect(JSON.parse(await readFile(statePath, 'utf8'))).toEqual({
      browserosId: id,
    })
  })

  function mkTempDir(): string {
    const dir = mkdtempSync(join(tmpdir(), 'browseros-identity-test-'))
    tempDirs.push(dir)
    return dir
  }
})
