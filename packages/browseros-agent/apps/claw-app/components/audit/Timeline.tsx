import { ChevronDown, ChevronRight, Image as ImageIcon } from 'lucide-react'
import { useState } from 'react'
import { AspectRatio } from '@/components/ui/aspect-ratio'
import { cn } from '@/lib/utils'
import {
  type ToolDispatchRow,
  taskScreenshotUrl,
} from '@/modules/api/audit.hooks'
import { parseResultMeta } from '@/screens/audit/audit.helpers'

interface TimelineProps {
  dispatches: ToolDispatchRow[]
  startedAt: number
  endEvent: {
    createdAt: number
    kind: 'closed' | 'errored'
    reason: string | null
  } | null
  onScreenshotClick: (dispatchId: number) => void
}

const HIGH_RISK_TOOLS = new Set(['act', 'evaluate', 'run', 'download'])

export function Timeline({
  dispatches,
  startedAt,
  endEvent,
  onScreenshotClick,
}: TimelineProps) {
  return (
    <section className="rounded-2xl border border-border-2 bg-card p-4">
      <header className="flex items-baseline justify-between pb-3">
        <h2 className="font-semibold text-ink-1">Timeline</h2>
        <span className="text-[12.5px] text-ink-3">
          {dispatches.length} event{dispatches.length === 1 ? '' : 's'}
        </span>
      </header>
      <ol className="space-y-1.5">
        {dispatches.map((d) => (
          <TimelineRow
            key={d.id}
            dispatch={d}
            offsetMs={Math.max(0, d.createdAt - startedAt)}
            onScreenshotClick={onScreenshotClick}
          />
        ))}
        <SessionEndRow startedAt={startedAt} endEvent={endEvent} />
      </ol>
    </section>
  )
}

interface TimelineRowProps {
  dispatch: ToolDispatchRow
  offsetMs: number
  onScreenshotClick: (dispatchId: number) => void
}

function TimelineRow({
  dispatch,
  offsetMs,
  onScreenshotClick,
}: TimelineRowProps) {
  const highRisk = HIGH_RISK_TOOLS.has(dispatch.toolName)
  const [expanded, setExpanded] = useState(highRisk)
  const meta = parseResultMeta(dispatch.resultMeta)
  const isError = meta?.isError ?? false
  const isScreenshot = dispatch.toolName === 'screenshot' && !isError
  return (
    <li
      className={cn(
        'rounded-lg border border-transparent px-2 py-1.5 transition hover:border-border-2 hover:bg-bg-sunken',
        highRisk && 'border-amber-500/30 bg-amber-500/5',
        isError && 'border-red-500/30 bg-red-500/5',
      )}
    >
      <button
        type="button"
        onClick={() => setExpanded((v) => !v)}
        className="grid w-full grid-cols-[auto_5rem_minmax(0,1fr)_auto_auto] items-center gap-3 text-left"
      >
        {expanded ? (
          <ChevronDown className="size-3.5 text-ink-3" />
        ) : (
          <ChevronRight className="size-3.5 text-ink-3" />
        )}
        <span className="font-mono text-[11.5px] text-ink-3">
          T+{formatOffset(offsetMs)}
        </span>
        <div className="flex min-w-0 items-center gap-2">
          <span className="font-mono font-semibold text-[12.5px] text-ink-1">
            {dispatch.toolName}
          </span>
          <span className="truncate text-[12.5px] text-ink-3">
            {argsSummary(dispatch.argsJson)}
          </span>
        </div>
        <span className="font-mono text-[11.5px] text-ink-3">
          {dispatch.durationMs ?? 0}ms
        </span>
        {highRisk && (
          <span className="rounded bg-amber-500/15 px-1.5 py-0.5 font-semibold text-[10.5px] text-amber-700 uppercase tracking-wide dark:text-amber-300">
            High risk
          </span>
        )}
      </button>
      {expanded && (
        <div className="mt-2 space-y-2 border-border-2 border-t px-1 pt-2">
          {dispatch.argsJson && (
            <Block label="args">
              <pre className="overflow-x-auto whitespace-pre-wrap break-all text-[11.5px]">
                {dispatch.argsJson}
              </pre>
            </Block>
          )}
          {dispatch.resultMeta && (
            <Block label="result">
              <pre className="overflow-x-auto whitespace-pre-wrap break-all text-[11.5px]">
                {dispatch.resultMeta}
              </pre>
            </Block>
          )}
          {isScreenshot && (
            <Block label="screenshot">
              <button
                type="button"
                onClick={() => onScreenshotClick(dispatch.id)}
                className="block w-64 overflow-hidden rounded-md border border-border-2"
              >
                <AspectRatio ratio={16 / 10}>
                  <img
                    src={taskScreenshotUrl(dispatch.id)}
                    alt={`Screenshot at T+${formatOffset(offsetMs)}`}
                    className="h-full w-full object-cover"
                    loading="lazy"
                  />
                </AspectRatio>
              </button>
            </Block>
          )}
          {dispatch.url && (
            <Block label="page">
              <a
                href={dispatch.url}
                target="_blank"
                rel="noreferrer"
                className="text-[12.5px] text-accent hover:underline"
              >
                {dispatch.url}
              </a>
            </Block>
          )}
          {!dispatch.argsJson &&
            !dispatch.resultMeta &&
            !isScreenshot &&
            !dispatch.url && (
              <div className="text-[12px] text-ink-3">
                <ImageIcon className="mr-1 inline size-3" />
                No extra detail recorded.
              </div>
            )}
        </div>
      )}
    </li>
  )
}

function Block({
  label,
  children,
}: {
  label: string
  children: React.ReactNode
}) {
  return (
    <div className="space-y-1">
      <div className="font-mono font-semibold text-[10.5px] text-ink-3 uppercase tracking-wide">
        {label}
      </div>
      <div className="rounded-md bg-bg-sunken p-2">{children}</div>
    </div>
  )
}

function SessionEndRow({
  startedAt,
  endEvent,
}: {
  startedAt: number
  endEvent: TimelineProps['endEvent']
}) {
  if (!endEvent) {
    return (
      <li className="flex items-center gap-3 px-2 py-1.5 text-[12.5px] text-ink-3">
        <span className="inline-block size-2 animate-pulse rounded-full bg-accent" />
        Still running, no session-close received yet.
      </li>
    )
  }
  const offset = Math.max(0, endEvent.createdAt - startedAt)
  return (
    <li className="flex items-center gap-3 px-2 py-1.5 text-[12.5px] text-ink-3">
      <span className="inline-block size-2 rounded-full bg-ink-3" />
      <span className="font-mono">T+{formatOffset(offset)}</span>
      <span>
        session{' '}
        {endEvent.kind === 'closed'
          ? 'closed'
          : `errored (${endEvent.reason ?? 'unknown'})`}
      </span>
    </li>
  )
}

function formatOffset(ms: number): string {
  if (ms < 1000) return `${ms}ms`
  const seconds = ms / 1000
  if (seconds < 60) return `${seconds.toFixed(2)}s`
  const totalSec = Math.floor(seconds)
  const mins = Math.floor(totalSec / 60)
  const rem = totalSec % 60
  return `${mins}m${rem.toString().padStart(2, '0')}s`
}

function argsSummary(argsJson: string | null): string {
  if (!argsJson || argsJson === '{}') return ''
  if (argsJson.length <= 80) return argsJson
  return `${argsJson.slice(0, 80)}…`
}
