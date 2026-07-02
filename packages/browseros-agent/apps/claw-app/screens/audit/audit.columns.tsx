import type { ColumnDef } from '@tanstack/react-table'
import { ChevronRight } from 'lucide-react'
import { AgentDot } from '@/components/audit/AgentDot'
import type { TaskSummary } from '@/modules/api/audit.hooks'
import {
  abbreviateSequence,
  formatDuration,
  formatRelative,
} from './audit.helpers'

/**
 * Module-level column array. Per tanstack-table v8 docs, `columns`
 * must be a stable reference across renders; otherwise the table
 * re-builds its internal column tree every render. Defining this
 * outside the component is the canonical stable-reference recipe.
 *
 * Editorial cockpit language: mono tabular numerics for grid data,
 * agent dot + mono-uppercase label for identity, LIVE / FAILED
 * folded inline into the agent cell so the row's identity carries
 * its state (DONE stays silent, no Status column).
 */
export const TASK_COLUMNS: ColumnDef<TaskSummary>[] = [
  {
    id: 'agent',
    header: 'Agent',
    accessorKey: 'agentLabel',
    cell: ({ row }) => (
      <div className="inline-flex items-center gap-2">
        <AgentDot slug={row.original.slug} />
        <span className="font-mono text-[11px] text-ink-2 uppercase tracking-[0.06em]">
          {row.original.agentLabel}
        </span>
        {row.original.status === 'live' && <LiveInlineChip />}
        {row.original.status === 'failed' && <FailedInlineChip />}
      </div>
    ),
    enableSorting: true,
  },
  {
    id: 'title',
    header: 'Title',
    accessorKey: 'title',
    cell: ({ row }) => (
      <span className="block truncate text-[13px] text-ink-1">
        {row.original.title}
      </span>
    ),
    enableSorting: false,
  },
  {
    id: 'sequence',
    header: 'Tools used',
    accessorFn: (t) => t.toolSequence.join('/'),
    cell: ({ row }) => (
      <span className="block max-w-[240px] truncate font-mono text-[11.5px] text-ink-3">
        {abbreviateSequence(row.original.toolSequence)}
      </span>
    ),
    enableSorting: false,
  },
  {
    id: 'tools',
    header: 'Actions',
    accessorFn: (t) => t.dispatchCount,
    cell: ({ getValue }) => (
      <span className="font-mono text-[11.5px] text-ink-2 tabular-nums">
        {getValue<number>()}
      </span>
    ),
    enableSorting: true,
  },
  {
    id: 'duration',
    header: 'Dur.',
    accessorFn: (t) => t.durationMs,
    cell: ({ getValue }) => (
      <span className="font-mono text-[11.5px] text-ink-2 tabular-nums">
        {formatDuration(getValue<number>())}
      </span>
    ),
    sortingFn: 'basic',
    enableSorting: true,
  },
  {
    id: 'when',
    header: 'When',
    accessorKey: 'startedAt',
    cell: ({ getValue }) => (
      <span className="font-mono text-[11.5px] text-ink-3 tabular-nums">
        {formatRelative(getValue<number>(), Date.now())}
      </span>
    ),
    enableSorting: true,
  },
  {
    id: 'chevron',
    header: '',
    cell: () => (
      <ChevronRight
        className="size-3.5 text-ink-4 opacity-0 transition-opacity group-hover:opacity-100"
        aria-hidden
      />
    ),
    enableSorting: false,
  },
]

/**
 * Column ids whose cells + headers should be right-aligned. Used by
 * the Audit screen wrapper to decorate `<TableHead>` / `<TableCell>`
 * with `text-right`. Kept as a single source of truth so header +
 * cell alignment cannot drift.
 */
export const NUMERIC_COLUMN_IDS = new Set([
  'tools',
  'duration',
  'when',
  'chevron',
])

function LiveInlineChip() {
  return (
    <span className="inline-flex items-center gap-1 font-mono text-[10px] text-accent uppercase tracking-[0.08em]">
      <span
        aria-hidden
        className="inline-block size-1.5 animate-[pulse-dot_1.4s_ease-in-out_infinite] rounded-full bg-accent shadow-[0_0_6px_hsl(130_46%_33%/0.6)]"
      />
      LIVE
    </span>
  )
}

function FailedInlineChip() {
  return (
    <span className="inline-flex items-center gap-1 font-mono text-[10px] text-red-500 uppercase tracking-[0.08em]">
      <span
        aria-hidden
        className="inline-block size-1.5 rounded-full bg-red-500"
      />
      FAILED
    </span>
  )
}
