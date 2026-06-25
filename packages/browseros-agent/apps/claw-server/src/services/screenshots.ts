/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * On-disk store for per-dispatch screenshot bytes. Files live at
 * `<browserosDir>/claw-server/screenshots/<dispatchId>.jpg` and are
 * served by the audit screenshot route via Bun.file(). Writes are
 * fire-and-forget; a hiccup logs at warn and never blocks the agent.
 *
 * SQLite stores only the dispatch row plus a result_meta summary;
 * the JPEG bytes live on disk so the audit DB stays small and the
 * stream path is a plain file send.
 */

import { mkdirSync } from 'node:fs'
import { writeFile } from 'node:fs/promises'
import { dirname } from 'node:path'
import { resolveClawServerPath } from '../lib/browseros-dir'
import { logger } from '../lib/logger'

export function screenshotPath(dispatchId: number): string {
  return resolveClawServerPath('screenshots', `${dispatchId}.jpg`)
}

export interface PersistScreenshotInput {
  dispatchId: number
  toolName: string
  result: {
    isError: boolean
    structuredContent: unknown
  }
}

/**
 * Fire-and-forget. Never throws.
 * No-op for non-screenshot tools, error results, or results without
 * a base64 image string.
 */
export function persistScreenshot(input: PersistScreenshotInput): void {
  if (input.toolName !== 'screenshot') return
  if (input.result.isError) return
  const bytes = extractImageBytes(input.result.structuredContent)
  if (!bytes) return
  const path = screenshotPath(input.dispatchId)
  try {
    mkdirSync(dirname(path), { recursive: true })
  } catch (err) {
    logger.warn('screenshot dir create failed', {
      dispatchId: input.dispatchId,
      error: err instanceof Error ? err.message : String(err),
    })
    return
  }
  void writeFile(path, bytes).catch((err) => {
    logger.warn('screenshot write failed', {
      dispatchId: input.dispatchId,
      error: err instanceof Error ? err.message : String(err),
    })
  })
}

function extractImageBytes(structured: unknown): Buffer | null {
  if (!structured || typeof structured !== 'object') return null
  const sc = structured as Record<string, unknown>
  const image = sc.image
  if (typeof image !== 'string' || image.length === 0) return null
  try {
    return Buffer.from(image, 'base64')
  } catch {
    return null
  }
}
