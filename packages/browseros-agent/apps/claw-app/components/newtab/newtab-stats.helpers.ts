import type { CockpitStatsWindow } from '@browseros/claw-api'

export interface StatGlanceStrings {
  tokensSaved: string
  timeSaved: string
  percentFewer: string
}

const compactNumberFormat = new Intl.NumberFormat('en-US', {
  compactDisplay: 'short',
  maximumFractionDigits: 1,
  notation: 'compact',
})

function formatHumanTime(milliseconds: number): string {
  const totalMinutes = Math.floor(Math.max(0, milliseconds) / 60_000)
  const hours = Math.floor(totalMinutes / 60)
  const minutes = totalMinutes % 60
  return hours === 0
    ? `${minutes}m`
    : `${hours}h ${String(minutes).padStart(2, '0')}m`
}

/** Derives the compact glance strings (tokens saved, time saved, percent fewer). */
export function formatStatGlance(
  windowStats: CockpitStatsWindow,
): StatGlanceStrings {
  const percentFewer =
    windowStats.screenshotFirstTokenEstimate > 0
      ? Math.round(
          ((windowStats.screenshotFirstTokenEstimate -
            windowStats.browserClawTokenEstimate) /
            windowStats.screenshotFirstTokenEstimate) *
            100,
        )
      : 0
  return {
    tokensSaved: compactNumberFormat.format(
      Math.max(0, windowStats.rawTokenSavingsEstimate),
    ),
    timeSaved: formatHumanTime(windowStats.humanTimeSavedMs),
    percentFewer: `${Math.max(0, percentFewer)}%`,
  }
}
