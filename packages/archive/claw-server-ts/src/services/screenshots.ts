/** Session-owned screenshot storage backed by audited dispatch ownership. */

import { existsSync, mkdirSync, readFileSync } from 'node:fs'
import { rename, rm, writeFile } from 'node:fs/promises'
import { dirname } from 'node:path'
import type { SessionScreenshot } from '@browseros/claw-api'
import { and, asc, eq } from 'drizzle-orm'
import { resolveClawServerPath } from '../lib/browserclaw-dir'
import { logger } from '../lib/logger'
import { getAuditDb } from '../modules/db/db'
import { toolDispatches } from '../modules/db/schema/tool-dispatches.sql'

export function sessionScreenshotPath(
  sessionId: string,
  screenshotId: number,
): string {
  return resolveClawServerPath(
    'screenshots',
    safeSessionKey(sessionId),
    `${screenshotId.toString()}.jpg`,
  )
}

export function legacyScreenshotPath(screenshotId: number): string {
  return resolveClawServerPath('screenshots', `${screenshotId.toString()}.jpg`)
}

export function hasSessionScreenshotFile(
  sessionId: string,
  screenshotId: number,
): boolean {
  return (
    existsSync(sessionScreenshotPath(sessionId, screenshotId)) ||
    existsSync(legacyScreenshotPath(screenshotId))
  )
}

export async function writeSessionScreenshot(
  sessionId: string,
  screenshotId: number,
  bytes: Uint8Array,
): Promise<boolean> {
  const path = sessionScreenshotPath(sessionId, screenshotId)
  const pendingPath = `${path}.${crypto.randomUUID()}.pending`
  try {
    mkdirSync(dirname(path), { recursive: true })
    // Readers treat final-path existence as screenshot metadata, so publish
    // only after every byte has reached a sibling file on the same volume.
    await writeFile(pendingPath, bytes)
    await rename(pendingPath, path)
    return true
  } catch (error) {
    await rm(pendingPath, { force: true }).catch(() => undefined)
    logger.warn('session screenshot write failed', {
      sessionId,
      screenshotId,
      error: error instanceof Error ? error.message : String(error),
    })
    return false
  }
}

export function listSessionScreenshots(sessionId: string): SessionScreenshot[] {
  return getAuditDb()
    .select({
      screenshotId: toolDispatches.id,
      capturedAt: toolDispatches.createdAt,
      toolName: toolDispatches.toolName,
    })
    .from(toolDispatches)
    .where(eq(toolDispatches.sessionId, sessionId))
    .orderBy(asc(toolDispatches.createdAt), asc(toolDispatches.id))
    .all()
    .filter((row) => hasSessionScreenshotFile(sessionId, row.screenshotId))
}

export function readSessionScreenshot(
  sessionId: string,
  screenshotId: number,
): Uint8Array | null {
  const owned = getAuditDb()
    .select({ id: toolDispatches.id })
    .from(toolDispatches)
    .where(
      and(
        eq(toolDispatches.sessionId, sessionId),
        eq(toolDispatches.id, screenshotId),
      ),
    )
    .get()
  if (!owned) return null

  const scopedPath = sessionScreenshotPath(sessionId, screenshotId)
  const path = existsSync(scopedPath)
    ? scopedPath
    : legacyScreenshotPath(screenshotId)
  if (!existsSync(path)) {
    logger.warn('session screenshot file missing', {
      sessionId,
      screenshotId,
    })
    return null
  }
  try {
    return readFileSync(path)
  } catch (error) {
    logger.warn('session screenshot read failed', {
      sessionId,
      screenshotId,
      error: error instanceof Error ? error.message : String(error),
    })
    return null
  }
}

function safeSessionKey(sessionId: string): string {
  return `s-${Buffer.from(sessionId, 'utf8').toString('base64url')}`
}

export const screenshotStorageForTesting = {
  safeSessionKey,
}
