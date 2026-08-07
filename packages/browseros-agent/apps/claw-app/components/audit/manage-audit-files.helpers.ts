/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Helpers for the "Manage audit files" dialog: human-readable byte sizes and
 * the mapping between the retention Select and the API policy.
 */

import type { AuditRetention } from '@browseros/claw-api'

/** Human-readable size in MB (below 1 GB) or GB. */
export function formatBytes(bytes: number): string {
  const value = Number.isFinite(bytes) && bytes > 0 ? bytes : 0
  const gb = value / 1024 ** 3
  if (gb >= 1) return `${gb.toFixed(2)} GB`
  const mb = value / 1024 ** 2
  return `${mb.toFixed(mb >= 10 || mb === 0 ? 0 : 1)} MB`
}

/** The retention dropdown options. `custom` reveals a days input. */
export type RetentionChoice = '7' | '30' | 'custom' | 'never'

/** Maps a persisted policy back to the dropdown selection + days input. */
export function choiceFromRetention(retention: AuditRetention): {
  choice: RetentionChoice
  days: number
} {
  if (retention.mode === 'keepForever') return { choice: 'never', days: 30 }
  const days = retention.days ?? 7
  if (days === 7) return { choice: '7', days }
  if (days === 30) return { choice: '30', days }
  return { choice: 'custom', days }
}

/** Builds the API request body from the dropdown selection + custom days. */
export function retentionRequest(
  choice: RetentionChoice,
  customDays: number,
): AuditRetention {
  switch (choice) {
    case 'never':
      return { mode: 'keepForever' }
    case '7':
      return { mode: 'deleteAfterDays', days: 7 }
    case '30':
      return { mode: 'deleteAfterDays', days: 30 }
    case 'custom':
      return {
        mode: 'deleteAfterDays',
        days: Math.max(1, Math.floor(customDays)),
      }
  }
}

/** Confirmation copy for the "Clean up now" gate. */
export function cleanupConfirmText(retention: AuditRetention): string {
  if (retention.mode === 'keepForever') {
    return 'Retention is set to Never, so this only removes orphaned audit files and reclaims disk space. Continue?'
  }
  const days = retention.days ?? 7
  return `This permanently deletes all audit data, screenshots, and replays older than ${days} days, then reclaims the disk space. Continue?`
}
