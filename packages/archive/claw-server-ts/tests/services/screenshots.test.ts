import { afterEach, beforeEach, describe, expect, it } from 'bun:test'
import { mkdirSync, writeFileSync } from 'node:fs'
import { dirname } from 'node:path'
import {
  getAuditDb,
  resetAuditDbForTesting,
  setAuditDbForTesting,
} from '../../src/modules/db/db'
import { toolDispatches } from '../../src/modules/db/schema/tool-dispatches.sql'
import {
  legacyScreenshotPath,
  listSessionScreenshots,
  readSessionScreenshot,
  screenshotStorageForTesting,
  sessionScreenshotPath,
  writeSessionScreenshot,
} from '../../src/services/screenshots'
import { withTempBrowserClawDir } from '../_helpers/temp-browserclaw-dir'

function seedDispatch(
  sessionId: string,
  createdAt: number,
  toolName: string,
): number {
  return getAuditDb()
    .insert(toolDispatches)
    .values({
      createdAt,
      agentId: `agent-${sessionId}`,
      slug: 'codex',
      agentLabel: 'Codex',
      sessionId,
      toolName,
    })
    .returning({ id: toolDispatches.id })
    .get().id
}

describe('session screenshot storage', () => {
  beforeEach(() => setAuditDbForTesting())
  afterEach(() => resetAuditDbForTesting())

  it('encodes every session id into one path-safe storage segment', () => {
    for (const sessionId of [
      'session-live',
      '../../escape',
      'slashes/and\\backslashes',
      '',
    ]) {
      const key = screenshotStorageForTesting.safeSessionKey(sessionId)
      expect(key.startsWith('s-')).toBe(true)
      expect(key).not.toContain('/')
      expect(key).not.toContain('\\')
      expect(key).not.toContain('..')
    }
  })

  it('writes scoped bytes and denies cross-session reads', async () => {
    await withTempBrowserClawDir(async () => {
      const id = seedDispatch('session-a', 100, 'navigate')
      const bytes = new Uint8Array([0xff, 0xd8, 0xff, 0xd9])

      expect(await writeSessionScreenshot('session-a', id, bytes)).toBe(true)
      expect(sessionScreenshotPath('session-a', id)).not.toBe(
        sessionScreenshotPath('session-b', id),
      )
      expect(readSessionScreenshot('session-a', id)).toEqual(bytes)
      expect(readSessionScreenshot('session-b', id)).toBeNull()
      expect(readSessionScreenshot('session-a', id + 1)).toBeNull()
    })
  })

  it('lists existing screenshots oldest-first with id tie-breaking', async () => {
    await withTempBrowserClawDir(async () => {
      const laterId = seedDispatch('session-a', 200, 'act')
      const firstTieId = seedDispatch('session-a', 100, 'navigate')
      const secondTieId = seedDispatch('session-a', 100, 'snapshot')
      const foreignId = seedDispatch('session-b', 50, 'read')
      const missingId = seedDispatch('session-a', 300, 'tabs')
      const bytes = new Uint8Array([0xff, 0xd8])
      for (const id of [laterId, firstTieId, secondTieId, foreignId]) {
        const sessionId = id === foreignId ? 'session-b' : 'session-a'
        await writeSessionScreenshot(sessionId, id, bytes)
      }

      expect(listSessionScreenshots('session-a')).toEqual([
        { screenshotId: firstTieId, capturedAt: 100, toolName: 'navigate' },
        { screenshotId: secondTieId, capturedAt: 100, toolName: 'snapshot' },
        { screenshotId: laterId, capturedAt: 200, toolName: 'act' },
      ])
      expect(listSessionScreenshots('session-a')).not.toContainEqual(
        expect.objectContaining({ screenshotId: missingId }),
      )
    })
  })

  it('reads legacy flat files only after audit ownership succeeds', async () => {
    await withTempBrowserClawDir(async () => {
      const id = seedDispatch('session-a', 100, 'snapshot')
      const path = legacyScreenshotPath(id)
      mkdirSync(dirname(path), { recursive: true })
      writeFileSync(path, new Uint8Array([0xff, 0xd8, 1]))

      expect(readSessionScreenshot('session-a', id)).toEqual(
        new Uint8Array([0xff, 0xd8, 1]),
      )
      expect(readSessionScreenshot('session-b', id)).toBeNull()
    })
  })
})
