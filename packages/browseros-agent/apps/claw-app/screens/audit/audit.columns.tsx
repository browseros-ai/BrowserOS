import type { ColumnDef } from '@tanstack/react-table'
import { ChevronRight } from 'lucide-react'
import { AgentDot } from '@/components/audit/AgentDot'
import { StatusBadge } from '@/components/audit/StatusBadge'
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
 * Relative-time formatting reads Date.now() inside the cell so the
 * column array never has to take a `now` prop (which would
 * destabilise the reference). This is fine because cells re-render
 * whenever data changes; the time is always fresh enough for the
 * "Xs ago" granularity.
 */
export const TASK_COLUMNS: ColumnDef<TaskSummary>[] = [
  {
    id: 'agent',
    header: 'Agent',
    accessorKey: 'agentLabel',
    cell: ({ row }) => (
      <div className="flex items-center gap-2">
        <AgentDot slug={row.original.slug} />
        <span className="font-medium text-ink-1">
          {row.original.agentLabel}
        </span>
      </div>
    ),
    enableSorting: true,
  },
  {
    id: 'title',
    header: 'Title',
    accessorKey: 'title',
    cell: ({ row }) => (
      <div className="flex flex-col leading-tight">
        <span className="font-medium text-ink-1">{row.original.title}</span>
        <span className="text-[11.5px] text-ink-3">
          {abbreviateSequence(row.original.toolSequence)}
        </span>
      </div>
    ),
    enableSorting: false,
  },
  {
    id: 'tools',
    header: 'Tools',
    accessorFn: (t) => t.dispatchCount,
    cell: ({ getValue }) => (
      <span className="font-mono text-[12.5px] text-ink-2">
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
      <span className="font-mono text-[12.5px] text-ink-2">
        {formatDuration(getValue<number>())}
      </span>
    ),
    sortingFn: 'basic',
    enableSorting: true,
  },
  {
    id: 'status',
    header: 'Status',
    accessorKey: 'status',
    cell: ({ getValue }) => (
      <StatusBadge status={getValue<TaskSummary['status']>()} />
    ),
    enableSorting: true,
  },
  {
    id: 'when',
    header: 'When',
    accessorKey: 'startedAt',
    cell: ({ getValue }) => (
      <span className="text-[12.5px] text-ink-3">
        {formatRelative(getValue<number>(), Date.now())}
      </span>
    ),
    enableSorting: true,
  },
  {
    id: 'chevron',
    header: '',
    cell: () => <ChevronRight className="size-4 text-ink-3" aria-hidden />,
    enableSorting: false,
  },
]
