import { afterEach, describe, expect, it } from 'bun:test'
import { mkdtemp, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'
import { resolveComposeResourcePath } from '../../../../src/api/services/openclaw/openclaw-service'

describe('resolveComposeResourcePath', () => {
  let tempDir: string | null = null

  afterEach(async () => {
    if (tempDir) {
      await rm(tempDir, { recursive: true, force: true })
      tempDir = null
    }
  })

  it('prefers the packaged resourcesDir copy when present', async () => {
    tempDir = await mkdtemp(join(tmpdir(), 'openclaw-compose-resource-'))
    const resourcesDir = join(tempDir, 'resources')
    const composePath = join(resourcesDir, 'openclaw-compose.yml')
    await Bun.write(composePath, 'services:\n')

    expect(resolveComposeResourcePath(resourcesDir)).toBe(composePath)
  })

  it('falls back to the source tree when no packaged copy exists', () => {
    expect(resolveComposeResourcePath(undefined)).toBe(
      resolve(import.meta.dir, '../../../../resources/openclaw-compose.yml'),
    )
  })
})
