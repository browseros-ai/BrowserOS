import type { ColumnDef } from '@tanstack/react-table'
import { cn } from '@/lib/utils'
import type { TaskSummary } from '@/modules/api/audit.hooks'
import {
  abbreviateSequence,
  formatDuration,
  formatRelative,
  formatTokensCompact,
  formatTokensFull,
} from './audit.helpers'

/**
 * Module-level column array. Per tanstack-table v8 docs, `columns`
 * must be a stable reference across renders; otherwise the table
 * re-builds its internal column tree every render. Defining this
 * outside the component is the canonical stable-reference recipe.
 *
 * Ledger language: a cobalt header bar over a white card, agent
 * identity as a blue link, the target in near-black at reading size,
 * and the supporting grid data in the muted blue ink scale. Rows are
 * not individually actionable — the whole row navigates — so there is
 * no trailing affordance column. LIVE / FAILED / STOPPED fold inline
 * into the agent cell; DONE stays silent, which is the common case.
 */
export const TASK_COLUMNS: ColumnDef<TaskSummary>[] = [
  {
    id: 'agent',
    header: 'Agent',
    accessorKey: 'agentLabel',
    cell: ({ row }) => (
      <div className="flex items-center gap-2">
        <span className="min-w-0 truncate text-[12px] text-ledger-link">
          {row.original.label}
        </span>
        {row.original.status === 'live' && <LiveInlineChip />}
        {row.original.status === 'failed' && <FailedInlineChip />}
        {row.original.status === 'cancelled' && <StoppedInlineChip />}
      </div>
    ),
    enableSorting: false,
  },
  {
    id: 'title',
    header: 'Target',
    accessorKey: 'title',
    // system-ui rather than the app's Schibsted Grotesk sans: the target is
    // the one cell a reader scans rather than skims, and the design calls
    // for the platform UI face at reading size here.
    cell: ({ row }) => (
      <div className="min-w-0">
        <span className="block truncate font-[system-ui,sans-serif] text-[13px] text-ledger-ink">
          {row.original.name}
        </span>
        {row.original.taskSummary && (
          <span className="mt-0.5 line-clamp-2 block text-[11px] text-ledger-ink-2 leading-snug">
            {row.original.taskSummary}
          </span>
        )}
      </div>
    ),
    enableSorting: false,
  },
  {
    id: 'sequence',
    header: 'Tool chain',
    accessorFn: (t) => t.toolSequence.join('/'),
    // Four links, not the shared default of five: five overflows the 240px
    // track for almost every real chain, and a CSS clip mid-word reads as a
    // rendering bug next to the explicit trailing ellipsis. The wider
    // cockpit tiles keep the default.
    cell: ({ row }) => (
      <span className="block truncate text-[12px] text-ledger-ink-3">
        {abbreviateSequence(row.original.toolSequence, 4)}
      </span>
    ),
    enableSorting: false,
  },
  {
    id: 'tokens',
    header: 'Tokens',
    accessorFn: (t) => t.tokenUsage?.totalTokenEstimate,
    cell: ({ row }) => <TokensCell task={row.original} />,
    enableSorting: false,
  },
  {
    id: 'duration',
    header: 'Duration',
    accessorFn: (t) => t.durationMs,
    cell: ({ getValue }) => (
      <span className="text-[12px] text-ledger-ink-2 tabular-nums">
        {formatDuration(getValue<number>())}
      </span>
    ),
    enableSorting: false,
  },
  {
    id: 'when',
    header: 'When',
    accessorKey: 'startedAt',
    cell: ({ getValue }) => (
      <span className="text-[12px] text-ledger-ink-3 tabular-nums">
        {formatRelative(getValue<number>(), Date.now())}
      </span>
    ),
    enableSorting: false,
  },
]

/**
 * Column ids whose cells + headers are right-aligned. Used by the Audit
 * screen wrapper to decorate `<TableHead>` / `<TableCell>` with
 * `text-right`. Kept as a single source of truth so header + cell
 * alignment cannot drift.
 */
export const NUMERIC_COLUMN_IDS = new Set(['tokens', 'duration', 'when'])

/**
 * Per-column track widths for the `table-fixed` ledger. Every column is
 * pinned except `title`, which absorbs the remaining width — that is what
 * makes the truncation in the fixed columns land on a predictable edge
 * instead of drifting with content.
 *
 * These are TRACK widths, so each one is the design's content width plus
 * the 16px of cell padding around it (`CELL_PADDING` in Audit.tsx splits
 * that as 8px + 8px, or 16px on the card's outer edges). Sizing them to
 * the bare content width instead would shave 16px off every column and
 * push the tool chain into permanent mid-word clipping.
 */
export const COLUMN_WIDTHS: Record<string, string> = {
  agent: 'w-[260px]',
  sequence: 'w-[288px]',
  tokens: 'w-[88px]',
  duration: 'w-[88px]',
  when: 'w-[88px]',
}

/**
 * Session token consumption. Shows a compact total ("12.3k") with the exact
 * count on hover; renders an em dash for legacy/unmeasured sessions whose wire
 * payload omits `tokenUsage`.
 */
function TokensCell({ task }: { task: TaskSummary }) {
  const usage = task.tokenUsage
  if (!usage) {
    return (
      <span
        className="text-[12px] text-ledger-ink-3 tabular-nums"
        title="Token usage not measured for this session"
      >
        —
      </span>
    )
  }
  return (
    <span
      className="text-[12px] text-ledger-ink-2 tabular-nums"
      title={`${formatTokensFull(usage.totalTokenEstimate)} tokens`}
    >
      {formatTokensCompact(usage.totalTokenEstimate)}
    </span>
  )
}

/**
 * Exceptional-state pills. One geometry for all three so they read as a
 * family; only Live carries a saturated fill. No dot and no animation —
 * a run being live is a state to notice, not an alarm to chase.
 */
const STATE_CHIP =
  '-my-[3px] inline-flex shrink-0 items-center rounded-full px-2.5 py-[3px] font-semibold text-[11px] leading-[14px]'

function LiveInlineChip() {
  return (
    <span
      className={cn(STATE_CHIP, 'bg-cyanotype-live text-cyanotype-live-ink')}
    >
      Live
    </span>
  )
}

function FailedInlineChip() {
  return <span className={cn(STATE_CHIP, 'bg-red-tint text-red')}>Failed</span>
}

function StoppedInlineChip() {
  return (
    <span className={cn(STATE_CHIP, 'bg-card-tint text-ink-3')}>Stopped</span>
  )
}
