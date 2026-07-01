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
 * `wait`) are normally excluded from the fallback: back-to-back reads
 * would produce visually identical frames. EXCEPTION: the FIRST
 * successful dispatch on a given (agentId, pageId) pair within the
 * session writes even if the tool is read-only. This guarantees at
 * least one visual anchor per tab for audits that consist entirely
 * of reads (a common codex research pattern). Tracked by
 * `agentTabs.hasFirstCapture` / `markFirstCaptureDone`; the ledger
 * is dropped on session end via `cleanupSessionState`.
 *
 * The screencast cache remains ephemeral. The persistence here is a
 * single snapshot AT dispatch complete time, not a continuous
 * mirror of the cache to disk.
 */

import { mkdirSync } from 'node:fs'
import { writeFile } from 'node:fs/promises'
import { dirname } from 'node:path'
import { env } from '../env'
import { agentTabs } from '../lib/agent-tabs'
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
   * Page id for the dispatch. For most tools this is the args-derived
   * value from `extractPageId`. For `tabs new` the caller derives it
   * from `result.structuredContent.page` (only place the id exists).
   * Null when the tool does not target a specific page (`tab_groups`,
   * `windows`, `run`); the fallback path skips when this is null.
   */
  pageId: number | null
  /**
   * Agent id resolved from the MCP client identity. Needed for the
   * first-capture-per-page policy so we can track per-agent-per-page
   * whether the audit already has a visual anchor for this tab.
   * Null when identity resolution failed; the first-capture override
   * cannot fire without an agentId (falls back to strict deny-list).
   */
  agentId: string | null
  result: {
    isError: boolean
    content?: unknown
    structuredContent?: unknown
  }
}

/**
 * Tools that read from the current page state without mutating it.
 * Excluded from the screencast fallback by default: back-to-back reads
 * produce visually identical frames, so writing one per dispatch is
 * pure waste. Overridden by the first-capture policy so the FIRST
 * read-only dispatch on a given tab still fires (see the docstring
 * at the top of this file).
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
    if (input.agentId !== null && input.pageId !== null) {
      agentTabs.markFirstCaptureDone(input.agentId, input.pageId)
    }
    return
  }

  // Branch 2: screencast cache fallback. Guarded by an env flag so
  // operators can revert to strict behaviour without a code change.
  if (!env.screencastScreenshotFallback) return
  if (input.pageId === null) return

  if (READ_ONLY_TOOLS.has(input.toolName)) {
    // Read-only tools are skipped by default. EXCEPT: if this is
    // the first successful capture for this (agent, page) pair,
    // we let the write through so the operator gets at least one
    // visual anchor per tab even in an all-read-only audit.
    if (input.agentId === null) return
    if (agentTabs.hasFirstCapture(input.agentId, input.pageId)) return
  }

  const frame = screencastCache.get(input.pageId)
  if (!frame || !frame.jpegBase64) return
  // Buffer.from(str, 'base64') never throws in Node/Bun; invalid
  // chars are silently skipped. Guard against a zero-length result
  // explicitly so we never write a 0-byte JPEG that renders as a
  // broken icon in the audit UI.
  const cacheBytes = Buffer.from(frame.jpegBase64, 'base64')
  if (cacheBytes.length === 0) return
  writeBytesToDisk(input.dispatchId, cacheBytes)
  if (input.agentId !== null) {
    agentTabs.markFirstCaptureDone(input.agentId, input.pageId)
  }
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
  // Buffer.from(str, 'base64') never throws; a zero-length result
  // means the tool sent garbage or empty base64. Treat as absent so
  // the fallback branch can decide whether to write the cache frame
  // instead of writing a 0-byte file here.
  const bytes = Buffer.from(image, 'base64')
  return bytes.length > 0 ? bytes : null
}
