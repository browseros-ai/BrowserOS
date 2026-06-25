import {
  Check,
  ChevronDown,
  ChevronRight,
  ChevronsDownUp,
  ChevronsUpDown,
  Copy,
  Image as ImageIcon,
} from 'lucide-react'
import { useState } from 'react'
import { AspectRatio } from '@/components/ui/aspect-ratio'
import { Button } from '@/components/ui/button'
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

function defaultExpandedSet(dispatches: ToolDispatchRow[]): Set<number> {
  const ids = new Set<number>()
  for (const d of dispatches) {
    if (HIGH_RISK_TOOLS.has(d.toolName)) ids.add(d.id)
  }
  return ids
}

export function Timeline({
  dispatches,
  startedAt,
  endEvent,
  onScreenshotClick,
}: TimelineProps) {
  // Initial state: HIGH RISK rows pre-expanded. Lazy init so the
  // dispatch list is only walked once per mount; future polling
  // updates do not reset the user's manual toggles.
  const [expanded, setExpanded] = useState<Set<number>>(() =>
    defaultExpandedSet(dispatches),
  )
  const toggle = (id: number): void => {
    setExpanded((prev) => {
      const next = new Set(prev)
      if (next.has(id)) next.delete(id)
      else next.add(id)
      return next
    })
  }
  const expandAll = (): void =>
    setExpanded(new Set(dispatches.map((d) => d.id)))
  const collapseAll = (): void => setExpanded(new Set())
  const allExpanded =
    dispatches.length > 0 && dispatches.every((d) => expanded.has(d.id))
  const noneExpanded = expanded.size === 0

  return (
    <section className="rounded-2xl border border-border-2 bg-card p-4">
      <header className="flex items-center justify-between gap-3 pb-3">
        <h2 className="font-semibold text-ink-1">Timeline</h2>
        <div className="flex items-center gap-1.5">
          <Button
            type="button"
            variant="ghost"
            size="sm"
            onClick={expandAll}
            disabled={allExpanded || dispatches.length === 0}
            className="h-7 gap-1 px-2 text-[11.5px] text-ink-3 hover:text-ink-1"
            data-testid="timeline-expand-all"
          >
            <ChevronsUpDown className="size-3.5" />
            Expand all
          </Button>
          <Button
            type="button"
            variant="ghost"
            size="sm"
            onClick={collapseAll}
            disabled={noneExpanded}
            className="h-7 gap-1 px-2 text-[11.5px] text-ink-3 hover:text-ink-1"
            data-testid="timeline-collapse-all"
          >
            <ChevronsDownUp className="size-3.5" />
            Collapse all
          </Button>
          <span className="pl-2 text-[12.5px] text-ink-3">
            {dispatches.length} event{dispatches.length === 1 ? '' : 's'}
          </span>
        </div>
      </header>
      <ol className="space-y-1.5">
        {dispatches.map((d) => (
          <TimelineRow
            key={d.id}
            dispatch={d}
            offsetMs={Math.max(0, d.createdAt - startedAt)}
            expanded={expanded.has(d.id)}
            onToggle={() => toggle(d.id)}
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
  expanded: boolean
  onToggle: () => void
  onScreenshotClick: (dispatchId: number) => void
}

function TimelineRow({
  dispatch,
  offsetMs,
  expanded,
  onToggle,
  onScreenshotClick,
}: TimelineRowProps) {
  const highRisk = HIGH_RISK_TOOLS.has(dispatch.toolName)
  const meta = parseResultMeta(dispatch.resultMeta)
  const isError = meta?.isError ?? false
  const isScreenshot = dispatch.toolName === 'screenshot' && !isError
  return (
    <li
      className={cn(
        'rounded-lg border border-transparent px-2 py-1.5',
        highRisk && 'border-amber-500/30 bg-amber-500/5',
        isError && 'border-red-500/30 bg-red-500/5',
      )}
    >
      <button
        type="button"
        onClick={onToggle}
        className={cn(
          'grid w-full grid-cols-[auto_5rem_minmax(0,1fr)_auto_auto] items-center gap-3 rounded-md px-1 py-1 text-left transition-colors',
          // Hover-tint only the header, never the body. Otherwise the
          // tint matches the args / result codeblock backgrounds and the
          // codeblocks visually disappear into the row.
          'hover:bg-card-tint',
        )}
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
            <Block label="args" copyText={dispatch.argsJson}>
              <pre className="overflow-x-auto whitespace-pre-wrap break-all text-[11.5px]">
                {dispatch.argsJson}
              </pre>
            </Block>
          )}
          {dispatch.resultMeta && (
            <Block label="result" copyText={dispatch.resultMeta}>
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
            <Block label="page" copyText={dispatch.url}>
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
  copyText,
  children,
}: {
  label: string
  copyText?: string
  children: React.ReactNode
}) {
  const [copied, setCopied] = useState(false)
  const handleCopy = (): void => {
    if (!copyText) return
    void navigator.clipboard.writeText(copyText).then(() => {
      setCopied(true)
      setTimeout(() => setCopied(false), 1500)
    })
  }
  return (
    <div className="space-y-1">
      <div className="flex items-center justify-between gap-2">
        <div className="font-mono font-semibold text-[10.5px] text-ink-3 uppercase tracking-wide">
          {label}
        </div>
        {copyText && (
          <button
            type="button"
            onClick={handleCopy}
            className="inline-flex items-center gap-1 rounded px-1.5 py-0.5 font-mono text-[10.5px] text-ink-3 uppercase tracking-wide transition-colors hover:bg-card-tint hover:text-ink-1"
            aria-label={`Copy ${label}`}
            data-testid={`timeline-block-copy-${label}`}
          >
            {copied ? (
              <Check className="size-3" />
            ) : (
              <Copy className="size-3" />
            )}
            {copied ? 'copied' : 'copy'}
          </button>
        )}
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
