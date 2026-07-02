import {
  flexRender,
  getCoreRowModel,
  getSortedRowModel,
  type SortingState,
  useReactTable,
} from '@tanstack/react-table'
import { ArrowRight } from 'lucide-react'
import { Fragment, useMemo, useState } from 'react'
import { useLocation, useNavigate } from 'react-router'
import { AuditEmpty } from '@/components/audit/AuditEmpty'
import { AuditHoverPreview } from '@/components/audit/AuditHoverPreview'
import { FilterBar } from '@/components/audit/FilterBar'
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
import { NUMERIC_COLUMN_IDS, TASK_COLUMNS } from './audit.columns'
import { useAuditScreenData } from './audit.data'
import {
  formatDayHeading,
  isSameLocalDay,
  orderByLiveThenRecency,
} from './audit.helpers'

/**
 * Editorial audit screen. Preserves the tanstack-table + shadcn
 * Table primitives (sortable headers, keyboard nav, infinite
 * pagination) and restyles them to match the cockpit's language:
 * hairline-separated rows on the shell, mono tabular numerics for
 * grid data, agent-dot identity, LIVE / FAILED folded inline into
 * the agent cell, DONE silent. Day-of-week headings are injected
 * as spanning `<TableRow>` dividers between date groups when the
 * default `when`-descending sort is active. On row hover, a fixed
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
    setSort,
  } = useAuditScreenData()
  const navigate = useNavigate()
  const location = useLocation()

  const hasActiveFilters =
    filters.agentId !== null ||
    filters.status !== null ||
    filters.site !== null ||
    filters.search.length > 0

  // LIVE-first pre-sort so a running session floats to the top of
  // the initial view. Operator column sorts still work: they run
  // through tanstack-table's sortingState on top of this input.
  const orderedTasks = useMemo(() => orderByLiveThenRecency(tasks), [tasks])

  const sortId = filters.sort?.id
  const sortDesc = filters.sort?.desc
  const sorting = useMemo<SortingState>(
    () =>
      sortId !== undefined && sortDesc !== undefined
        ? [{ id: sortId, desc: sortDesc }]
        : [],
    [sortId, sortDesc],
  )
  const state = useMemo(() => ({ sorting }), [sorting])

  const table = useReactTable<TaskSummary>({
    data: orderedTasks,
    columns: TASK_COLUMNS,
    state,
    onSortingChange: (updater) => {
      const next = typeof updater === 'function' ? updater(sorting) : updater
      setSort(next[0] ?? null)
    },
    getCoreRowModel: getCoreRowModel(),
    getSortedRowModel: getSortedRowModel(),
  })

  // Day-of-week headings only make sense when rows are date-ordered.
  // If the operator picks a non-`when` sort (e.g. sort by duration
  // desc), skip the dividers so we do not scramble day groups.
  const activeSortId = sorting[0]?.id
  const showDayHeadings = !activeSortId || activeSortId === 'when'

  const [hoveredTask, setHoveredTask] = useState<TaskSummary | null>(null)
  const rows = table.getRowModel().rows
  const visibleColumnCount = table.getVisibleFlatColumns().length

  return (
    <div className="relative mx-auto flex w-full max-w-6xl flex-col gap-6 px-8 pt-8 pb-16">
      <header>
        <h1 className="font-extrabold text-3xl leading-tight tracking-tight md:text-4xl">
          Audit
        </h1>
      </header>

      {!isError && (tasks.length > 0 || hasActiveFilters) && (
        <FilterBar
          agentOptions={agentOptions}
          statusOptions={statusOptions}
          siteOptions={siteOptions}
          selectedAgentId={filters.agentId}
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
          <div className="relative" onMouseLeave={() => setHoveredTask(null)}>
            <Table>
              <TableHeader>
                {table.getHeaderGroups().map((hg) => (
                  <TableRow
                    key={hg.id}
                    className="border-border-2 border-b hover:bg-transparent"
                  >
                    {hg.headers.map((h) => {
                      const canSort = h.column.getCanSort()
                      const isNumeric = NUMERIC_COLUMN_IDS.has(h.column.id)
                      const sortDir = h.column.getIsSorted()
                      return (
                        <TableHead
                          key={h.id}
                          onClick={
                            canSort
                              ? h.column.getToggleSortingHandler()
                              : undefined
                          }
                          className={cn(
                            'font-mono text-[10.5px] text-ink-3 uppercase tracking-[0.08em]',
                            canSort && 'cursor-pointer select-none',
                            isNumeric && 'text-right',
                          )}
                        >
                          <span className="inline-flex items-center gap-1">
                            {h.isPlaceholder
                              ? null
                              : flexRender(
                                  h.column.columnDef.header,
                                  h.getContext(),
                                )}
                            {canSort && sortDir === 'asc' && (
                              <span aria-hidden>▲</span>
                            )}
                            {canSort && sortDir === 'desc' && (
                              <span aria-hidden>▼</span>
                            )}
                          </span>
                        </TableHead>
                      )
                    })}
                  </TableRow>
                ))}
              </TableHeader>
              <TableBody>
                {rows.map((row, idx) => {
                  const prev = idx > 0 ? rows[idx - 1] : null
                  // Null-check narrows prev so we do not need the
                  // non-null assertion inside isSameLocalDay's number
                  // parameters. Biome's noNonNullAssertion rule bans
                  // the `!` form; this preserves the type discipline.
                  const dayChanged =
                    showDayHeadings &&
                    (prev === null ||
                      !isSameLocalDay(
                        row.original.startedAt,
                        prev.original.startedAt,
                      ))
                  return (
                    <Fragment key={row.id}>
                      {dayChanged && (
                        <TableRow className="hover:bg-transparent">
                          <TableCell
                            colSpan={visibleColumnCount}
                            className="border-none pt-6 pb-1"
                          >
                            <span className="font-mono text-[10.5px] text-ink-3 uppercase tracking-[0.08em]">
                              {formatDayHeading(row.original.startedAt)}
                            </span>
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
                        className="group cursor-pointer border-border-2 border-t hover:bg-card-tint"
                      >
                        {row.getVisibleCells().map((cell) => (
                          <TableCell
                            key={cell.id}
                            className={cn(
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
          </div>
          {hasNextPage && (
            <div className="pt-2 text-center">
              <button
                type="button"
                onClick={fetchNextPage}
                disabled={isFetchingNextPage}
                className="group inline-flex items-center gap-1.5 font-mono text-[12px] text-ink-3 uppercase tracking-[0.08em] transition-colors hover:text-ink-1 disabled:cursor-not-allowed disabled:opacity-60"
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

interface TableShellProps {
  table: ReturnType<typeof useReactTable<TaskSummary>>
}

/**
 * Loading-state skeleton table shell. Renders the real header row
 * plus 6 empty `<TableRow>` shells so the layout does not jump when
 * data lands.
 */
function TableShell({ table }: TableShellProps) {
  return (
    <div className="relative">
      <Table>
        <TableHeader>
          {table.getHeaderGroups().map((hg) => (
            <TableRow
              key={hg.id}
              className="border-border-2 border-b hover:bg-transparent"
            >
              {hg.headers.map((h) => {
                const isNumeric = NUMERIC_COLUMN_IDS.has(h.column.id)
                return (
                  <TableHead
                    key={h.id}
                    className={cn(
                      'font-mono text-[10.5px] text-ink-3 uppercase tracking-[0.08em]',
                      isNumeric && 'text-right',
                    )}
                  >
                    {h.isPlaceholder
                      ? null
                      : flexRender(h.column.columnDef.header, h.getContext())}
                  </TableHead>
                )
              })}
            </TableRow>
          ))}
        </TableHeader>
        <TableBody>
          {['s1', 's2', 's3', 's4', 's5', 's6'].map((id) => (
            <TableRow
              key={id}
              className="border-border-2 border-t hover:bg-transparent"
            >
              <TableCell
                colSpan={table.getVisibleFlatColumns().length}
                className="py-4"
              >
                <div className="h-4 w-full animate-pulse rounded bg-card-tint" />
              </TableCell>
            </TableRow>
          ))}
        </TableBody>
      </Table>
    </div>
  )
}
