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
 *
 * Two branches feed the file:
 *
 *   1. Tool-result branch (original): the tool result carries base64
 *      image bytes (the explicit `screenshot` tool does this; some
 *      future variants may too). We decode + write.
 *   2. Screencast-fallback branch: for a page-targeted state-mutating
 *      dispatch that produced no image bytes, we snapshot the current
 *      screencast cache frame for the tab. Populates the audit with
 *      visual context for `navigate` / `act` / `tabs new` / etc. that
 *      would otherwise render as image-less rows.
 *
 * Read-only page-targeted tools (`snapshot`, `read`, `grep`, `diff`,
 * `wait`) are excluded from the fallback: back-to-back reads would
 * produce visually identical frames.
 *
 * The screencast cache remains ephemeral. The persistence here is a
 * single snapshot AT dispatch complete time, not a continuous
 * mirror of the cache to disk.
 */

import { mkdirSync } from 'node:fs'
import { writeFile } from 'node:fs/promises'
import { dirname } from 'node:path'
import { env } from '../env'
import { resolveClawServerPath } from '../lib/browseros-dir'
import { logger } from '../lib/logger'
import { screencastCache } from './screencast-cache'
import { extractToolResultImageData } from './tool-result-image'

export function screenshotPath(dispatchId: number): string {
  return resolveClawServerPath('screenshots', `${dispatchId}.jpg`)
}

export interface PersistScreenshotInput {
  dispatchId: number
  toolName: string
  /**
   * Page id for the dispatch, resolved via `extractPageId` at the
   * call site. Null when the tool does not target a specific page
   * (`tab_groups`, `windows`, `run`) OR when the dispatch is a
   * `tabs new` whose page id is only born in the result. The
   * fallback path skips when this is null.
   */
  pageId: number | null
  result: {
    isError: boolean
    content?: unknown
    structuredContent?: unknown
  }
}

/**
 * Tools that read from the current page state without mutating it.
 * Excluded from the screencast fallback: back-to-back reads produce
 * visually identical frames, so writing one per dispatch is pure
 * waste. Tools NOT in this set that ARE page-targeted (act, navigate,
 * tabs, evaluate, download, pdf, upload, screenshot) get the
 * fallback because they either change page state or the operator
 * wants a visual around the time they fired.
 */
const READ_ONLY_TOOLS: ReadonlySet<string> = new Set([
  'snapshot',
  'read',
  'grep',
  'diff',
  'wait',
])

/** Fire-and-forget. Never throws. */
export function persistScreenshot(input: PersistScreenshotInput): void {
  if (input.result.isError) return

  // Branch 1: tool result carries image bytes (explicit screenshot
  // tool, or any future tool that emits an image content block).
  const toolBytes = extractImageBytes(input.result)
  if (toolBytes) {
    writeBytesToDisk(input.dispatchId, toolBytes)
    return
  }

  // Branch 2: screencast cache fallback for page-targeted
  // state-mutating dispatches. Guarded by an env flag so operators
  // can revert to the strict behaviour without a code change.
  if (!env.screencastScreenshotFallback) return
  if (READ_ONLY_TOOLS.has(input.toolName)) return
  if (input.pageId == null) return
  const frame = screencastCache.get(input.pageId)
  if (!frame) return
  let cacheBytes: Buffer
  try {
    cacheBytes = Buffer.from(frame.jpegBase64, 'base64')
  } catch {
    return
  }
  writeBytesToDisk(input.dispatchId, cacheBytes)
}

function writeBytesToDisk(dispatchId: number, bytes: Buffer): void {
  const path = screenshotPath(dispatchId)
  try {
    mkdirSync(dirname(path), { recursive: true })
  } catch (err) {
    logger.warn('screenshot dir create failed', {
      dispatchId,
      error: err instanceof Error ? err.message : String(err),
    })
    return
  }
  void writeFile(path, bytes).catch((err) => {
    logger.warn('screenshot write failed', {
      dispatchId,
      error: err instanceof Error ? err.message : String(err),
    })
  })
}

function extractImageBytes(
  result: PersistScreenshotInput['result'],
): Buffer | null {
  const image = extractToolResultImageData(result)
  if (!image) return null
  try {
    return Buffer.from(image, 'base64')
  } catch {
    return null
  }
}
