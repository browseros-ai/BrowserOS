import type { CockpitStats } from '@browseros/claw-api'
import { formatStatGlance } from './newtab-stats.helpers'

interface StatGlanceProps {
  stats: CockpitStats | undefined
  pending: boolean
}

/** The idle-line stat form: "45.1K tokens saved, 36m saved, 43% fewer". */
export function StatGlance({ stats, pending }: StatGlanceProps) {
  if (pending) return <GlanceSkeleton className="w-56" />
  if (!stats?.hasMeasuredStats) return null
  const glance = formatStatGlance(stats.allTime)
  return (
    <span className="text-ink-2 text-sm">
      <Num>{glance.tokensSaved}</Num> tokens saved,{' '}
      <Num>{glance.timeSaved}</Num> saved, <Num>{glance.percentFewer}</Num>{' '}
      fewer
    </span>
  )
}

/** The compact header form: "45.1K saved, 43% fewer". */
export function StatGlanceInline({ stats, pending }: StatGlanceProps) {
  if (pending) return <GlanceSkeleton className="w-36" />
  if (!stats?.hasMeasuredStats) return null
  const glance = formatStatGlance(stats.allTime)
  return (
    <span className="text-ink-2 text-sm">
      <Num>{glance.tokensSaved}</Num> saved, <Num>{glance.percentFewer}</Num>{' '}
      fewer
    </span>
  )
}

function Num({ children }: { children: string }) {
  return <span className="font-mono text-ink tabular-nums">{children}</span>
}

function GlanceSkeleton({ className }: { className: string }) {
  return (
    <span
      aria-hidden
      className={`inline-block h-3.5 animate-pulse rounded bg-card-tint align-middle ${className}`}
    />
  )
}
