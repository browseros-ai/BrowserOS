import {
  AlertTriangle,
  Check,
  CheckCircle2,
  ChevronRight,
  ExternalLink,
  History,
  Lock,
} from 'lucide-react'
import type { ComponentType, SVGProps } from 'react'
import { Link } from 'react-router'
import { cn } from '@/lib/utils'
import type { ActivityRow as ActivityRowData } from '@/modules/api/activity.hooks'

interface ActivityRowProps {
  row: ActivityRowData
}

interface ActivityStatusMeta {
  label: string
  textClass: string
  bgClass: string
  borderClass: string
  iconBgClass: string
  icon: ComponentType<SVGProps<SVGSVGElement>>
}

const STATUS_META: Record<ActivityRowData['status'], ActivityStatusMeta> = {
  blocked: {
    label: 'Blocked',
    textClass: 'text-red',
    bgClass: 'bg-red-tint',
    borderClass: 'border-red/20',
    iconBgClass: 'bg-card',
    icon: Lock,
  },
  'needs-human': {
    label: 'Needs you',
    textClass: 'text-amber',
    bgClass: 'bg-amber-tint',
    borderClass: 'border-amber/30',
    iconBgClass: 'bg-card',
    icon: AlertTriangle,
  },
  'needs-ok': {
    label: 'Needs your OK',
    textClass: 'text-amber',
    bgClass: 'bg-amber-tint',
    borderClass: 'border-amber/30',
    iconBgClass: 'bg-card',
    icon: AlertTriangle,
  },
  allowed: {
    label: 'Allowed',
    textClass: 'text-green',
    bgClass: 'bg-card',
    borderClass: 'border-border-2',
    iconBgClass: 'bg-green-tint',
    icon: Check,
  },
  running: {
    label: 'Running',
    textClass: 'text-green',
    bgClass: 'bg-card',
    borderClass: 'border-border-2',
    iconBgClass: 'bg-green-tint',
    icon: AlertTriangle,
  },
  done: {
    label: 'Done',
    textClass: 'text-ink-3',
    bgClass: 'bg-card',
    borderClass: 'border-border-2',
    iconBgClass: 'bg-bg-sunken',
    icon: CheckCircle2,
  },
}

/**
 * Status rows that require human attention (blocked, needs-human,
 * needs-ok) render a primary jump-to action ("Resolve" / "Take over"
 * / "Review"). Done and allowed rows just hint with a chevron — they
 * are informational, not actionable from the cockpit.
 */
const JUMP_LABEL: Partial<Record<ActivityRowData['status'], string>> = {
  blocked: 'Resolve',
  'needs-human': 'Take over',
  'needs-ok': 'Review',
}

function isFlagged(status: ActivityRowData['status']): boolean {
  return (
    status === 'blocked' || status === 'needs-human' || status === 'needs-ok'
  )
}

export function ActivityRow({ row }: ActivityRowProps) {
  const meta = STATUS_META[row.status]
  const flagged = isFlagged(row.status)
  const Icon = meta.icon
  const jumpLabel = JUMP_LABEL[row.status]

  return (
    <div
      className={cn(
        'flex items-start gap-3 rounded-xl border px-3.5 py-3 transition hover:border-border-strong hover:shadow-card',
        meta.bgClass,
        meta.borderClass,
      )}
    >
      <span
        className={cn(
          'relative mt-0.5 flex size-7 shrink-0 items-center justify-center rounded-lg',
          meta.iconBgClass,
          meta.textClass,
          flagged && 'border',
          flagged && meta.borderClass,
        )}
      >
        <Icon className="size-4" />
        <span
          aria-hidden
          className="absolute right-[-3px] bottom-[-3px] size-3 rounded-full border-2 border-card"
          style={{ backgroundColor: row.color }}
        />
      </span>
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-2">
          <span className="truncate font-bold text-[12.5px]">
            {row.agentLabel}
          </span>
          <span
            className={cn(
              'shrink-0 rounded-full bg-card px-1.5 py-[1.5px] font-bold text-[9.5px] uppercase tracking-wider',
              meta.textClass,
            )}
          >
            {meta.label}
          </span>
          <div className="flex-1" />
          <span className="shrink-0 text-[11px] text-ink-4">{row.when}</span>
        </div>
        <div className="mt-0.5 text-[12.5px] text-ink-2 leading-relaxed">
          {row.action}
          {row.site && (
            <>
              {' . '}
              <span className="font-mono text-[11.5px] text-ink-3">
                {row.site}
              </span>
            </>
          )}
          {row.toolCount !== undefined && row.toolCount > 0 && (
            <>
              {' . '}
              <span className="font-mono text-[11.5px] text-ink-3">
                {row.toolCount} {row.toolCount === 1 ? 'action' : 'actions'}
              </span>
            </>
          )}
        </div>
        {row.trail && (
          <div
            className="mt-0.5 truncate font-mono text-[10.5px] text-ink-3"
            title={row.trail}
          >
            {row.trail}
          </div>
        )}
      </div>
      {flagged && jumpLabel ? (
        <button
          type="button"
          className={cn(
            'inline-flex shrink-0 items-center gap-1.5 self-center whitespace-nowrap rounded-md border bg-card px-2.5 py-1.5 font-bold text-[12px] transition hover:brightness-95',
            meta.textClass,
            meta.borderClass,
          )}
        >
          {jumpLabel}
          <ExternalLink className="size-3.5" />
        </button>
      ) : row.status === 'done' && row.runId ? (
        <Link
          to={`/governance/audit/${row.runId}/replay`}
          className="inline-flex shrink-0 items-center gap-1.5 self-center whitespace-nowrap rounded-md border border-border-2 bg-card px-2.5 py-1.5 font-bold text-[12px] text-accent-ink transition hover:brightness-95"
        >
          <History className="size-3.5" />
          Replay
        </Link>
      ) : (
        <ChevronRight className="size-4 shrink-0 self-center text-ink-4" />
      )}
    </div>
  )
}
