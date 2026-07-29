import type { CockpitStats, CockpitStatsWindow } from '@browseros/claw-api'
import { ArrowRight } from 'lucide-react'
import { NavLink } from 'react-router'
import { Skeleton } from '@/components/ui/skeleton'

interface StatGlanceProps {
  stats?: CockpitStats
  isPending: boolean
}

const compactFormat = new Intl.NumberFormat('en-US', {
  compactDisplay: 'short',
  maximumFractionDigits: 1,
  notation: 'compact',
})

/** One legible line of the value stat, with a link to the full breakdown. */
export function StatGlance({ stats, isPending }: StatGlanceProps) {
  return (
    <div className="flex flex-wrap items-center gap-x-3 gap-y-2 text-sm">
      <span className="inline-flex shrink-0 items-center gap-1.5 text-ink-3">
        <span
          aria-hidden
          className="inline-block size-1.5 rounded-full bg-ink-4"
        />
        Idle
      </span>
      {isPending ? (
        <Skeleton className="h-4 w-56" />
      ) : stats?.hasMeasuredStats ? (
        <span className="min-w-0 text-ink-2">
          {summarizeWindow(stats.allTime)}
        </span>
      ) : (
        <span className="text-ink-3">No agent stats yet</span>
      )}
      <NavLink
        to="/audit"
        className="group ml-auto inline-flex shrink-0 items-center gap-1 font-mono text-[11px] text-ink-3 uppercase tracking-[0.08em] transition-colors hover:text-ink"
      >
        stats
        <ArrowRight className="size-3 transition-transform group-hover:translate-x-0.5" />
      </NavLink>
    </div>
  )
}

function summarizeWindow(window: CockpitStatsWindow): string {
  const tokens = compactFormat.format(
    Math.max(0, window.rawTokenSavingsEstimate),
  )
  const time = formatHumanTime(window.humanTimeSavedMs)
  const percent = Math.round(savingsRatio(window) * 100)
  return `${tokens} tokens · ${time} saved, ${percent}% fewer`
}

function savingsRatio(window: CockpitStatsWindow): number {
  if (window.screenshotFirstTokenEstimate <= 0) return 0
  const ratio =
    window.rawTokenSavingsEstimate / window.screenshotFirstTokenEstimate
  return Math.min(1, Math.max(0, ratio))
}

function formatHumanTime(milliseconds: number): string {
  const totalMinutes = Math.floor(Math.max(0, milliseconds) / 60_000)
  const hours = Math.floor(totalMinutes / 60)
  const minutes = totalMinutes % 60
  return hours === 0
    ? `${minutes}m`
    : `${hours}h ${String(minutes).padStart(2, '0')}m`
}
