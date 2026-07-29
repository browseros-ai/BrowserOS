import type { CockpitStatsWindow } from '@browseros/claw-api'

const compactNumberFormat = new Intl.NumberFormat('en-US', {
  compactDisplay: 'short',
  maximumFractionDigits: 1,
  notation: 'compact',
})

export function formatTokens(value: number): string {
  return compactNumberFormat.format(Math.max(0, value))
}

export function formatTimeSaved(milliseconds: number): string {
  const totalMinutes = Math.floor(Math.max(0, milliseconds) / 60_000)
  const hours = Math.floor(totalMinutes / 60)
  const minutes = totalMinutes % 60
  return hours === 0
    ? `${minutes}m`
    : `${hours}h ${String(minutes).padStart(2, '0')}m`
}

export function savingsPercent(window: CockpitStatsWindow): number {
  const total = window.screenshotFirstTokenEstimate
  if (total <= 0) return 0
  const ratio = Math.min(1, Math.max(0, window.rawTokenSavingsEstimate / total))
  return Math.round(ratio * 100)
}

export interface StatGlanceParts {
  tokensSaved: string
  timeSaved: string
  percentFewer: number
}

/** The compact value line: tokens saved, human time saved, percent fewer. */
export function formatStatGlance(window: CockpitStatsWindow): StatGlanceParts {
  return {
    tokensSaved: formatTokens(window.rawTokenSavingsEstimate),
    timeSaved: formatTimeSaved(window.humanTimeSavedMs),
    percentFewer: savingsPercent(window),
  }
}
