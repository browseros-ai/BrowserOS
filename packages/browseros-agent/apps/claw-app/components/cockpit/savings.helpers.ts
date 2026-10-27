/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { CockpitStats, CockpitStatsWindow } from '@browseros/claw-api'
import { roundTimeSaved } from '@/screens/cockpit/cockpit.helpers'

export type SavingsWindowKey = 'allTime' | 'last30Days' | 'last7Days'

export const SAVINGS_WINDOWS: readonly {
  key: SavingsWindowKey
  label: string
}[] = [
  { key: 'allTime', label: 'All time' },
  { key: 'last30Days', label: '30 days' },
  { key: 'last7Days', label: '7 days' },
]

export interface UsageComparison {
  /** Tokens the agents used with neo. */
  withNeo: number
  /** The estimate for the same tasks done from screenshots. */
  withoutNeo: number
  /** The difference: tokens neo kept the agents from using. */
  saved: number
  percentLess: number
}

/** AI token usage with and without neo, or null when there is no saving to show. */
export function usageComparison(
  stats: CockpitStats,
  window: CockpitStatsWindow,
): UsageComparison | null {
  const withNeo = Math.max(0, window.browserClawTokenEstimate)
  const withoutNeo = Math.max(0, window.screenshotFirstTokenEstimate)
  if (!stats.hasMeasuredStats || withoutNeo === 0 || withNeo >= withoutNeo) {
    return null
  }
  const percentLess = Math.round((1 - withNeo / withoutNeo) * 100)
  return percentLess > 0
    ? { withNeo, withoutNeo, saved: withoutNeo - withNeo, percentLess }
    : null
}

/** A bar's width against the longest one, never so thin it disappears. */
export function barWidth(value: number, longest: number): string {
  if (longest <= 0) return '0%'
  const percent = Math.min(100, Math.max(2, (value / longest) * 100))
  return `${Number(percent.toFixed(1))}%`
}

const compactTokens = new Intl.NumberFormat('en', {
  notation: 'compact',
  maximumFractionDigits: 1,
})

export function formatTokens(tokens: number): string {
  return compactTokens.format(Math.max(0, tokens))
}

/** "30 hours" or "12 minutes"; "Under a minute" before the first full one. */
export function timeSavedAmount(ms: number): string {
  const saved = roundTimeSaved(ms)
  if (!saved) return 'Under a minute'
  const unit = saved.amount === 1 ? saved.unit : `${saved.unit}s`
  return `${saved.amount.toLocaleString()} ${unit}`
}
