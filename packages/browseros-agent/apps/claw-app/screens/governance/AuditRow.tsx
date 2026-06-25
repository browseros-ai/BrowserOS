import { ChevronRight, Code, History, Sparkles } from 'lucide-react'
import { StatusBadge } from '@/components/cockpit/StatusBadge'
import { cn } from '@/lib/utils'
import type { RunRow } from '@/modules/api/runs.hooks'
import { auditIconFor, isCheckmarkHarness } from './governance.helpers'

interface AuditRowProps {
  run: RunRow
  onReplay: (run: RunRow) => void
}

export function AuditRow({ run, onReplay }: AuditRowProps) {
  const { Icon: StatusIcon, className: statusIconClass } = auditIconFor(
    run.status,
  )
  const HarnessIcon = isCheckmarkHarness(run.harness) ? Sparkles : Code
  return (
    <button
      type="button"
      onClick={() => onReplay(run)}
      className="group flex w-full items-center gap-3 rounded-xl border border-border bg-card p-3.5 text-left transition-shadow hover:border-border-strong hover:shadow-sm"
    >
      <StatusIcon className={cn('size-4 shrink-0', statusIconClass)} />
      <div className="min-w-0 flex-1">
        <div className="mb-1 flex items-center gap-2">
          <span className="inline-flex min-w-0 items-center gap-1.5 truncate font-bold text-ink text-sm">
            <HarnessIcon
              className={cn(
                'size-3 shrink-0',
                isCheckmarkHarness(run.harness) ? 'text-accent' : 'text-ink-3',
              )}
            />
            {run.agentLabel}
          </span>
          <StatusBadge status={run.status} />
        </div>
        <div className="flex flex-wrap items-center gap-x-1.5 gap-y-0.5 text-ink-3 text-xs">
          <span className="max-w-[340px] truncate text-ink-2">{run.title}</span>
          <span className="text-ink-4">·</span>
          <span>{run.when}</span>
          <span className="text-ink-4">·</span>
          <span>{run.actions} actions</span>
          {run.duration && (
            <>
              <span className="text-ink-4">·</span>
              <span>{run.duration}</span>
            </>
          )}
          {run.site && (
            <>
              <span className="text-ink-4">·</span>
              <span className="font-mono">{run.site}</span>
            </>
          )}
        </div>
      </div>
      <span className="inline-flex shrink-0 items-center gap-1 font-semibold text-accent-ink text-xs opacity-0 transition-opacity group-hover:opacity-100">
        <History className="size-3.5" />
        Replay
      </span>
      <ChevronRight className="size-4 shrink-0 text-ink-4" />
    </button>
  )
}
