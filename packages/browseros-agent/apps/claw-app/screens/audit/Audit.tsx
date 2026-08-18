import {
  flexRender,
  getCoreRowModel,
  useReactTable,
} from '@tanstack/react-table'
import { ArrowRight } from 'lucide-react'
import { Fragment, type ReactNode, useMemo, useState } from 'react'
import { useLocation, useNavigate } from 'react-router'
import { AuditEmpty } from '@/components/audit/AuditEmpty'
import { AuditHoverPreview } from '@/components/audit/AuditHoverPreview'
import { FilterBar } from '@/components/audit/FilterBar'
import { ManageAuditFilesDialog } from '@/components/audit/ManageAuditFilesDialog'
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@/components/ui/table'
import { cn } from '@/lib/utils'
import type { TaskSummary } from '@/modules/api/audit.hooks'
import {
  COLUMN_WIDTHS,
  NUMERIC_COLUMN_IDS,
  TASK_COLUMNS,
} from './audit.columns'
import { useAuditScreenData } from './audit.data'
import {
  formatDayHeading,
  isSameLocalDay,
  orderByLiveThenRecency,
} from './audit.helpers'

/** Keeps the implemented token column dormant without discarding its cell behavior. */
const SHOW_TOKEN_USAGE_COLUMN = false

/** First cell gets the card's left inset, last cell its right inset; the 8px
 *  each interior cell carries on both sides adds up to the 16px column gap. */
const CELL_PADDING = 'px-2 py-2.5 first:pl-4 last:pr-4'

/**
 * Audit ledger. Preserves the tanstack-table + shadcn Table primitives
 * and restyles them as a single bordered card: a quiet tinted header
 * strip, tinted day bands separating date groups, and hairline-separated
 * rows beneath. Rows are ordered LIVE-first then newest-first and are not
 * re-sortable — arbitrary column sorts would shred the day bands, and
 * the filter bar covers retrieval instead. On row hover, a fixed
 * top-right panel shows the session's screenshot preview.
 */
export function Audit() {
  const {
    tasks,
    agentOptions,
    statusOptions,
    siteOptions,
    isLoading,
    isError,
    hasNextPage,
    isFetchingNextPage,
    fetchNextPage,
    filters,
    setAgentFilter,
    setStatusFilter,
    setSiteFilter,
    setSearch,
  } = useAuditScreenData()
  const navigate = useNavigate()
  const location = useLocation()

  const hasActiveFilters =
    filters.agentSlug !== null ||
    filters.status !== null ||
    filters.site !== null ||
    filters.search.length > 0

  // LIVE-first pre-sort so a running session floats to the top of the
  // list regardless of when it started.
  const orderedTasks = useMemo(() => orderByLiveThenRecency(tasks), [tasks])

  const state = useMemo(
    () => ({ columnVisibility: { tokens: SHOW_TOKEN_USAGE_COLUMN } }),
    [],
  )

  const table = useReactTable<TaskSummary>({
    data: orderedTasks,
    columns: TASK_COLUMNS,
    state,
    getCoreRowModel: getCoreRowModel(),
  })

  const [hoveredTask, setHoveredTask] = useState<TaskSummary | null>(null)
  const rows = table.getRowModel().rows
  const visibleColumnCount = table.getVisibleFlatColumns().length

  return (
    <div className="relative mx-auto flex w-full max-w-6xl flex-col gap-6 px-8 pt-8 pb-16">
      <header className="flex items-start justify-between gap-4">
        <h1 className="font-extrabold text-3xl leading-tight tracking-tight md:text-4xl">
          Audit
        </h1>
        <ManageAuditFilesDialog />
      </header>

      {!isError && (tasks.length > 0 || hasActiveFilters) && (
        <FilterBar
          agentOptions={agentOptions}
          statusOptions={statusOptions}
          siteOptions={siteOptions}
          selectedAgentSlug={filters.agentSlug}
          selectedStatus={filters.status}
          selectedSite={filters.site}
          search={filters.search}
          onAgentChange={setAgentFilter}
          onStatusChange={setStatusFilter}
          onSiteChange={setSiteFilter}
          onSearchChange={setSearch}
        />
      )}

      {isError ? (
        <AuditEmpty variant="error" />
      ) : isLoading ? (
        <TableShell table={table} />
      ) : rows.length === 0 ? (
        <AuditEmpty variant={hasActiveFilters ? 'search-miss' : 'zero-tasks'} />
      ) : (
        <>
          {/* biome-ignore lint/a11y/noStaticElementInteractions: the onMouseLeave clears the supplementary hover-preview panel; the panel is a pointer-only progressive enhancement and does not gate any core information. */}
          <div onMouseLeave={() => setHoveredTask(null)}>
            <LedgerCard>
              <Table className="table-fixed">
                <LedgerHeader table={table} />
                <TableBody>
                  {rows.map((row, idx) => {
                    const prev = idx > 0 ? rows[idx - 1] : null
                    // Null-check narrows prev so we do not need the
                    // non-null assertion inside isSameLocalDay's number
                    // parameters. Biome's noNonNullAssertion rule bans
                    // the `!` form; this preserves the type discipline.
                    const dayChanged =
                      prev === null ||
                      !isSameLocalDay(
                        row.original.startedAt,
                        prev.original.startedAt,
                      )
                    return (
                      <Fragment key={row.id}>
                        {dayChanged && (
                          <TableRow className="border-none hover:bg-transparent">
                            <TableCell
                              colSpan={visibleColumnCount}
                              className="bg-ledger-band px-4 py-[7px] font-semibold text-[12px] text-ledger-band-ink"
                            >
                              {formatDayHeading(row.original.startedAt)}
                            </TableCell>
                          </TableRow>
                        )}
                        <TableRow
                          data-testid={`task-row-${row.original.sessionId}`}
                          onMouseEnter={() => setHoveredTask(row.original)}
                          onClick={() =>
                            navigate(
                              `/audit/${encodeURIComponent(row.original.sessionId)}`,
                              { state: { from: location.pathname } },
                            )
                          }
                          className="cursor-pointer border-ledger-divider border-b hover:bg-card-tint"
                        >
                          {row.getVisibleCells().map((cell) => (
                            <TableCell
                              key={cell.id}
                              className={cn(
                                CELL_PADDING,
                                NUMERIC_COLUMN_IDS.has(cell.column.id) &&
                                  'text-right',
                              )}
                            >
                              {flexRender(
                                cell.column.columnDef.cell,
                                cell.getContext(),
                              )}
                            </TableCell>
                          ))}
                        </TableRow>
                      </Fragment>
                    )
                  })}
                </TableBody>
              </Table>
            </LedgerCard>
          </div>
          {hasNextPage && (
            <div className="pt-2 text-center">
              <button
                type="button"
                onClick={fetchNextPage}
                disabled={isFetchingNextPage}
                className="group inline-flex items-center gap-1.5 font-medium text-[13px] text-ink-2 transition-colors hover:text-ink disabled:cursor-not-allowed disabled:opacity-60"
              >
                {isFetchingNextPage ? 'Loading...' : 'Load older tasks'}
                <ArrowRight className="size-3.5 transition-transform group-hover:translate-x-0.5" />
              </button>
            </div>
          )}
        </>
      )}

      <AuditHoverPreview task={hoveredTask} />
    </div>
  )
}

/**
 * White card the ledger sits in. `overflow-clip` is what rounds the
 * header strip's top corners and the last row's bottom corners: neither
 * element carries a radius of its own.
 */
function LedgerCard({ children }: { children: ReactNode }) {
  return (
    <div className="overflow-clip rounded-9 border border-ledger-border bg-card">
      {children}
    </div>
  )
}

interface LedgerTableProps {
  table: ReturnType<typeof useReactTable<TaskSummary>>
}

/** Quiet header strip. Labels only: the ledger is not operator-sortable. */
function LedgerHeader({ table }: LedgerTableProps) {
  return (
    <TableHeader>
      {table.getHeaderGroups().map((hg) => (
        <TableRow
          key={hg.id}
          className="border-ledger-border border-b bg-ledger-head hover:bg-ledger-head"
        >
          {hg.headers.map((h) => (
            <TableHead
              key={h.id}
              className={cn(
                CELL_PADDING,
                'h-auto font-medium text-[12px] text-ledger-head-ink',
                COLUMN_WIDTHS[h.column.id],
                NUMERIC_COLUMN_IDS.has(h.column.id) && 'text-right',
              )}
            >
              {h.isPlaceholder
                ? null
                : flexRender(h.column.columnDef.header, h.getContext())}
            </TableHead>
          ))}
        </TableRow>
      ))}
    </TableHeader>
  )
}

/**
 * Loading-state skeleton. Renders the real card + header bar plus 6
 * empty rows so the layout does not jump when data lands.
 */
function TableShell({ table }: LedgerTableProps) {
  return (
    <LedgerCard>
      <Table className="table-fixed">
        <LedgerHeader table={table} />
        <TableBody>
          {['s1', 's2', 's3', 's4', 's5', 's6'].map((id) => (
            <TableRow
              key={id}
              className="border-ledger-divider border-b hover:bg-transparent"
            >
              <TableCell
                colSpan={table.getVisibleFlatColumns().length}
                className="px-4 py-4"
              >
                <div className="h-4 w-full animate-pulse rounded bg-card-tint" />
              </TableCell>
            </TableRow>
          ))}
        </TableBody>
      </Table>
    </LedgerCard>
  )
}
