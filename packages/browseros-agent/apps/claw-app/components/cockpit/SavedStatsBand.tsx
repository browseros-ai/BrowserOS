import type { CockpitStats, CockpitStatsWindow } from '@browseros/claw-api'
import { useState } from 'react'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui/tabs'

export type { CockpitStats, CockpitStatsWindow } from '@browseros/claw-api'

interface SavedStatsBandProps {
  runningCount: number
  stats: CockpitStats
}

const WINDOWS = [
  { key: 'allTime', tabLabel: 'All time', valueLabel: 'all time' },
  { key: 'last30Days', tabLabel: '30 days', valueLabel: 'last 30 days' },
  { key: 'last7Days', tabLabel: '7 days', valueLabel: 'last 7 days' },
] as const

type WindowKey = (typeof WINDOWS)[number]['key']

const compactNumberFormat = new Intl.NumberFormat('en-US', {
  compactDisplay: 'short',
  maximumFractionDigits: 1,
  notation: 'compact',
})
const wholeNumberFormat = new Intl.NumberFormat('en-US', {
  maximumFractionDigits: 0,
})

export function SavedStatsBand({ runningCount, stats }: SavedStatsBandProps) {
  const [selectedWindow, setSelectedWindow] = useState<WindowKey>('allTime')

  if (!stats.hasMeasuredStats) return null

  return (
    <Tabs
      className="min-w-0 gap-4"
      data-saved-stats
      onValueChange={(value) => {
        if (isWindowKey(value)) setSelectedWindow(value)
      }}
      render={<section />}
      value={selectedWindow}
    >
      <header
        className="flex flex-wrap items-center gap-3"
        data-saved-stats-header
      >
        <h2 className="font-semibold text-[18px] text-cyanotype-ink leading-7">
          Since you started
        </h2>
        <span
          className="text-[12px] text-cyanotype-muted leading-4"
          data-running-status
        >
          {runningCount === 0 ? 'Nothing running' : `${runningCount} running`}
        </span>
        <TabsList
          activateOnFocus
          aria-label="Saved stats window"
          className="ml-auto h-9 rounded-[9px] bg-cyanotype-well p-1"
        >
          {WINDOWS.map(({ key, tabLabel }) => (
            <TabsTrigger
              className="h-7 flex-none rounded-md border-0 px-3 py-1 font-normal text-[12px] text-cyanotype-muted leading-4 shadow-none transition-[background-color,color,box-shadow] hover:text-cyanotype-ink data-active:bg-white data-active:font-semibold data-active:text-cyanotype-blue data-active:shadow-[0_1px_3px_rgba(12,39,66,0.12)] motion-reduce:transition-none"
              key={key}
              value={key}
            >
              {tabLabel}
            </TabsTrigger>
          ))}
        </TabsList>
      </header>

      {WINDOWS.map((windowDefinition) => (
        <TabsContent
          className="flex min-w-0 flex-col items-stretch gap-6 rounded-[9px] border border-cyanotype-border bg-card px-7 py-6 shadow-card md:flex-row md:items-center md:gap-8"
          data-saved-stats-card
          key={windowDefinition.key}
          value={windowDefinition.key}
        >
          <SavedStatsPanel
            windowDefinition={windowDefinition}
            windowStats={stats[windowDefinition.key]}
          />
        </TabsContent>
      ))}
    </Tabs>
  )
}

interface SavedStatsPanelProps {
  windowDefinition: (typeof WINDOWS)[number]
  windowStats: CockpitStatsWindow
}

function SavedStatsPanel({
  windowDefinition,
  windowStats,
}: SavedStatsPanelProps) {
  const visibleSavings = Math.max(0, windowStats.rawTokenSavingsEstimate)
  const savingsRatio = boundedRatio(
    windowStats.rawTokenSavingsEstimate,
    windowStats.screenshotFirstTokenEstimate,
  )
  const usedRatio = boundedRatio(
    windowStats.browserClawTokenEstimate,
    windowStats.screenshotFirstTokenEstimate,
  )

  return (
    <>
      <div className="min-w-0 flex-[2]" data-stats-primary>
        <div className="mb-4 flex flex-wrap items-end gap-x-3 gap-y-2">
          <div>
            <div className="mb-1.5 text-[12px] text-cyanotype-muted leading-4">
              Tokens saved · {windowDefinition.valueLabel}
            </div>
            <div
              className="font-extrabold text-[46px] text-cyanotype-ink tabular-nums leading-none tracking-[-0.03em]"
              data-stat="tokens-saved"
            >
              {formatCompact(visibleSavings)}
            </div>
          </div>
          <div
            className="inline-flex items-baseline gap-1.5 rounded-full border border-cyanotype-border bg-cyanotype-well px-3 py-1"
            data-savings-pill
          >
            <span
              className="font-extrabold text-[14px] text-cyanotype-blue tabular-nums leading-4"
              data-stat="percentage"
            >
              {Math.round(savingsRatio * 100)}%
            </span>
            <span className="text-[12px] text-cyanotype-blue leading-4">
              fewer tokens
            </span>
          </div>
        </div>

        {/* Keep both labels outside the bounded track so no ratio can overlap
            or clip them. */}
        <div className="mb-2 flex flex-wrap items-center justify-between gap-x-4 gap-y-1">
          <span className="inline-flex items-center gap-1.5 whitespace-nowrap font-semibold text-[12px] text-cyanotype-blue leading-4">
            used{' '}
            <span className="tabular-nums" data-stat="browserclaw-tokens">
              {formatCompact(windowStats.browserClawTokenEstimate)}
            </span>
          </span>
          <span className="text-[12px] text-cyanotype-muted leading-4">
            a screenshot-first agent would spend{' '}
            <span className="tabular-nums" data-stat="comparison-tokens">
              {formatCompact(windowStats.screenshotFirstTokenEstimate)}
            </span>
          </span>
        </div>

        <div
          aria-hidden
          className="relative h-3 min-w-0 overflow-hidden rounded-full bg-cyanotype-well shadow-[inset_0_0_0_1px_var(--color-cyanotype-border)]"
          data-budget-track
        >
          <div
            className="absolute inset-y-0 left-0 rounded-full bg-cyanotype-blue transition-[width] duration-300 motion-reduce:transition-none"
            data-used-fill
            style={{ width: `${usedRatio * 100}%` }}
          />
        </div>
        <p
          className="mt-2.5 text-[12px] text-cyanotype-soft leading-4"
          data-saved-stats-caption
        >
          compact DOM &amp; tool responses instead of a screenshot per call
        </p>
      </div>

      <div
        aria-hidden
        className="h-px w-full bg-cyanotype-border md:h-auto md:w-px md:self-stretch"
        data-stats-divider
      />

      <div className="flex min-w-0 flex-1 flex-col gap-5" data-stats-secondary>
        <div>
          <div className="mb-1.5 text-[12px] text-cyanotype-muted leading-4">
            Human time saved
          </div>
          <div
            className="font-extrabold text-[28px] text-cyanotype-ink tabular-nums leading-none tracking-[-0.02em]"
            data-stat="human-time"
          >
            {formatHumanTime(windowStats.humanTimeSavedMs)}
          </div>
        </div>
        <div>
          <div className="mb-1.5 text-[12px] text-cyanotype-muted leading-4">
            Sessions · tool calls
          </div>
          <div
            className="min-w-0 max-w-full break-all font-extrabold text-[28px] text-cyanotype-ink tabular-nums leading-none tracking-[-0.02em]"
            data-session-tool-metrics
          >
            <span data-stat="sessions">
              {formatWhole(windowStats.sessionCount)}
            </span>{' '}
            ·{' '}
            <span data-stat="tool-calls">
              {formatWhole(windowStats.toolCallCount)}
            </span>
          </div>
        </div>
      </div>
    </>
  )
}

function isWindowKey(value: unknown): value is WindowKey {
  return WINDOWS.some(({ key }) => key === value)
}

function boundedRatio(value: number, total: number): number {
  if (total <= 0) return 0
  return Math.min(1, Math.max(0, value / total))
}

function formatCompact(value: number): string {
  return compactNumberFormat.format(Math.max(0, value))
}

function formatWhole(value: number): string {
  return wholeNumberFormat.format(Math.max(0, value))
}

function formatHumanTime(milliseconds: number): string {
  const totalMinutes = Math.floor(Math.max(0, milliseconds) / 60_000)
  const hours = Math.floor(totalMinutes / 60)
  const minutes = totalMinutes % 60
  return hours === 0
    ? `${minutes}m`
    : `${hours}h ${String(minutes).padStart(2, '0')}m`
}
