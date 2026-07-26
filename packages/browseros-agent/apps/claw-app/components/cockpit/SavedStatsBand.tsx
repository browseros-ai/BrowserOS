import type { CockpitStats, CockpitStatsWindow } from '@browseros/claw-api'
import { useState } from 'react'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui/tabs'

export type { CockpitStats, CockpitStatsWindow } from '@browseros/claw-api'

interface SavedStatsBandProps {
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

export function SavedStatsBand({ stats }: SavedStatsBandProps) {
  const [selectedWindow, setSelectedWindow] = useState<WindowKey>('allTime')

  if (!stats.hasMeasuredStats) return null

  return (
    <Tabs
      className="min-w-0 gap-4"
      onValueChange={(value) => {
        if (isWindowKey(value)) setSelectedWindow(value)
      }}
      render={<section />}
      value={selectedWindow}
    >
      <header className="flex flex-wrap items-center gap-3">
        <h2 className="font-semibold text-ink text-lg">Since you started</h2>
        <span className="inline-flex items-center gap-2 font-mono text-[11px] text-ink-3 uppercase tracking-[0.08em]">
          <span
            aria-hidden
            className="inline-block size-1.5 rounded-full bg-ink-4"
          />
          nothing running
        </span>
        <TabsList
          activateOnFocus
          aria-label="Saved stats window"
          className="ml-auto h-auto bg-card-tint p-1"
        >
          {WINDOWS.map(({ key, tabLabel }) => (
            <TabsTrigger
              className="h-auto flex-none rounded-md border-0 px-3 py-1.5 font-mono font-normal text-[11px] text-ink-3 uppercase tracking-[0.06em] shadow-none transition-[background-color,color,box-shadow] hover:text-ink data-active:bg-card data-active:font-semibold data-active:text-accent-ink data-active:shadow-sm motion-reduce:transition-none"
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
          className="flex min-w-0 flex-col items-stretch gap-6 rounded-2xl border border-border-2 bg-card px-5 py-6 shadow-card md:flex-row md:items-center md:gap-8 md:px-7"
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
      <div className="min-w-0 flex-[2]">
        <div className="mb-4 flex flex-wrap items-end gap-x-3 gap-y-2">
          <div>
            <div className="mb-1.5 font-mono text-[10.5px] text-ink-3 uppercase tracking-[0.12em]">
              Tokens saved · {windowDefinition.valueLabel}
            </div>
            <div
              className="font-extrabold text-[46px] text-ink tabular-nums leading-none tracking-[-0.03em]"
              data-stat="tokens-saved"
            >
              {formatCompact(visibleSavings)}
            </div>
          </div>
          <div className="inline-flex items-baseline gap-1.5 rounded-full border border-accent-tint-2 bg-accent-tint px-3 py-1">
            <span
              className="font-extrabold text-accent-ink text-sm tabular-nums"
              data-stat="percentage"
            >
              {Math.round(savingsRatio * 100)}%
            </span>
            <span className="font-mono text-[10px] text-accent-ink uppercase tracking-[0.06em]">
              fewer tokens
            </span>
          </div>
        </div>

        {/* Usage gauge. The full bar is what a screenshot-first agent would
            spend; the accent fill is what BrowserClaw actually used. Both
            numbers live in the legend row above the bar — never absolutely
            positioned inside the bounded, overflow-hidden track — so they can
            never overlap or clip, at any used/total ratio or label length. The
            legend wraps to two lines before the labels would ever meet. */}
        <div className="mb-2 flex flex-wrap items-center justify-between gap-x-4 gap-y-1">
          <span className="inline-flex items-center gap-1.5 whitespace-nowrap font-mono font-semibold text-[11px] text-accent-ink tracking-[0.04em]">
            <span className="relative size-2.5 shrink-0">
              <span
                aria-hidden
                className="absolute inset-0 animate-ping rounded-full bg-accent/50 motion-reduce:animate-none"
                data-used-marker-ping
              />
              <span
                aria-hidden
                className="absolute inset-0 rounded-full bg-accent ring-2 ring-card"
              />
            </span>
            used{' '}
            <span className="tabular-nums" data-stat="browserclaw-tokens">
              {formatCompact(windowStats.browserClawTokenEstimate)}
            </span>
          </span>
          <span className="font-mono text-[11px] text-ink-3 tracking-[0.04em]">
            a screenshot-first agent would spend{' '}
            <span
              className="text-ink-2 tabular-nums"
              data-stat="comparison-tokens"
            >
              {formatCompact(windowStats.screenshotFirstTokenEstimate)}
            </span>
          </span>
        </div>

        <div
          aria-hidden
          className="relative h-3 min-w-0 overflow-hidden rounded-full bg-[repeating-linear-gradient(135deg,var(--color-card-tint),var(--color-card-tint)_9px,var(--color-card)_9px,var(--color-card)_10px)] shadow-[inset_0_0_0_1px_var(--color-border-2)]"
          data-budget-track
        >
          <div
            className="absolute inset-y-0 left-0 rounded-full bg-gradient-to-r from-accent to-accent-2 shadow-[0_2px_10px_color-mix(in_srgb,var(--color-accent)_45%,transparent)] transition-[width] duration-300 motion-reduce:transition-none"
            data-used-fill
            style={{ width: `${usedRatio * 100}%` }}
          />
        </div>
        <p className="mt-2.5 font-mono text-[10.5px] text-ink-4 tracking-[0.04em]">
          compact DOM &amp; tool responses instead of a screenshot per call
        </p>
      </div>

      <div aria-hidden className="h-px w-full bg-border md:h-auto md:w-px" />

      <div className="flex min-w-0 flex-1 flex-col gap-5">
        <div>
          <div className="mb-1.5 font-mono text-[10.5px] text-ink-3 uppercase tracking-[0.1em]">
            Human time saved
          </div>
          <div
            className="font-extrabold text-[28px] text-ink tabular-nums leading-none tracking-[-0.02em]"
            data-stat="human-time"
          >
            {formatHumanTime(windowStats.humanTimeSavedMs)}
          </div>
        </div>
        <div>
          <div className="mb-1.5 font-mono text-[10.5px] text-ink-3 uppercase tracking-[0.1em]">
            Sessions · tool calls
          </div>
          <div className="font-extrabold text-[28px] text-ink tabular-nums leading-none tracking-[-0.02em]">
            <span data-stat="sessions">
              {formatWhole(windowStats.sessionCount)}
            </span>{' '}
            <span className="font-bold text-base text-ink-4">
              ·{' '}
              <span data-stat="tool-calls">
                {formatWhole(windowStats.toolCallCount)}
              </span>
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
