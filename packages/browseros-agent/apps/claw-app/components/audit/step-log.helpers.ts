/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { describeStep } from '@/lib/step-sentences'
import type { ToolDispatchRow } from '@/modules/api/audit.hooks'

/** "1:05.42" from the start of the task, precise enough to tell close steps apart. */
export function preciseOffset(at: number, startedAt: number): string {
  const centis = Math.max(0, Math.floor((at - startedAt) / 10))
  const minutes = Math.floor(centis / 6000)
  const seconds = Math.floor((centis % 6000) / 100)
  const rest = centis % 100
  return `${minutes}:${String(seconds).padStart(2, '0')}.${String(rest).padStart(2, '0')}`
}

/** "260 ms" under a second, "1.2 s" above it, nothing when it was not measured. */
export function formatStepDuration(ms: number | undefined | null): string {
  if (ms == null || !Number.isFinite(ms) || ms < 0) return ''
  return ms < 1000 ? `${Math.round(ms)} ms` : `${(ms / 1000).toFixed(1)} s`
}

/** One log entry as plain text for copying: time, tool, sentence, then its details. */
export function logEntryText(row: ToolDispatchRow, startedAt: number): string {
  const head = [
    preciseOffset(row.createdAt, startedAt),
    row.toolName,
    describeStep(row),
    formatStepDuration(row.durationMs),
  ]
    .filter(Boolean)
    .join('  ')
  return [head, row.url, row.argsJson, row.resultMeta]
    .filter(Boolean)
    .join('\n')
}
