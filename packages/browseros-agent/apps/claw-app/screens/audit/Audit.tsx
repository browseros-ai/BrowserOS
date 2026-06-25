import {
  flexRender,
  getCoreRowModel,
  getSortedRowModel,
  type SortingState,
  useReactTable,
} from '@tanstack/react-table'
import { ScrollText } from 'lucide-react'
import { useMemo } from 'react'
import { useNavigate } from 'react-router'
import { FilterBar } from '@/components/audit/FilterBar'
import { EmptyState } from '@/components/cockpit/EmptyState'
import { Button } from '@/components/ui/button'
import { Skeleton } from '@/components/ui/skeleton'
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@/components/ui/table'
import type { TaskSummary } from '@/modules/api/audit.hooks'
import { TASK_COLUMNS } from './audit.columns'
import { useAuditScreenData } from './audit.data'

/**
 * Task-centric audit screen. Each MCP session becomes one row. Click
 * a row to navigate to its full timeline at `/audit/:sessionId`.
 * Filters round-trip through URL search params so browser back
 * restores the prior view.
 *
 * All of `useReactTable`'s options come from stable references
 * (module-level columns, useMemo'd data + state) so the table never
 * re-builds its core or sort row models when nothing relevant
 * changed. Per tanstack-table v8, passing fresh references on every
 * render forces an internal re-process each tick and the page
 * locks up under typing / filter cascades.
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

  // Mirror the URL-derived sort tuple into a memoised SortingState
  // array. Use the primitive id / desc as deps so the array reference
  // is stable across renders that did not change the sort (the
  // filters.sort object itself is rebuilt on every paramsToFilters
  // call even when nothing changed).
  const hasActiveFilters =
    filters.agentId !== null ||
    filters.status !== null ||
    filters.site !== null ||
    filters.search.length > 0
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
    data: tasks,
    columns: TASK_COLUMNS,
    state,
    onSortingChange: (updater) => {
      const next = typeof updater === 'function' ? updater(sorting) : updater
      setSort(next[0] ?? null)
    },
    getCoreRowModel: getCoreRowModel(),
    getSortedRowModel: getSortedRowModel(),
  })

  return (
    <div className="mx-auto flex w-full max-w-6xl flex-col gap-6 px-8 pt-10 pb-20">
      <header className="space-y-2">
        <div className="flex items-center gap-2.5">
          <span className="flex size-9 items-center justify-center rounded-xl bg-accent-tint text-accent">
            <ScrollText className="size-4.5" />
          </span>
          <div>
            <h1 className="font-extrabold text-2xl tracking-tight">Audit</h1>
            <p className="text-ink-3 text-sm">
              Tasks across every BrowserClaw session. Click a row to open its
              timeline.
            </p>
          </div>
        </div>
      </header>

      {/* Keep the FilterBar mounted across every state EXCEPT the
          first-load and error cases. Without this, a debounced search
          keystroke triggers a refetch -> isLoading flips true ->
          FilterBar unmounts -> the input the operator is typing into
          loses focus. The gate is intentionally permissive: as soon
          as either the user has tasks OR an active filter, the bar
          stays. */}
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

      {isLoading ? (
        <div className="space-y-2 rounded-2xl border border-border-2 bg-card p-4">
          {['s1', 's2', 's3', 's4', 's5', 's6'].map((id) => (
            <Skeleton key={id} className="h-10 w-full" />
          ))}
        </div>
      ) : isError ? (
        <EmptyState
          title="Could not load audit log"
          hint="Check that the cockpit server is running and the audit database is reachable."
        />
      ) : tasks.length === 0 ? (
        hasActiveFilters ? (
          <EmptyState
            title="No tasks match these filters"
            hint="Adjust the search or filter dropdowns above, or clear them to see every session again."
          />
        ) : (
          <EmptyState
            title="No tasks in this view"
            hint="Connect an agent via the MCP page and run a tool. Successful sessions land here within a few seconds."
          />
        )
      ) : (
        <div className="overflow-hidden rounded-2xl border border-border-2 bg-card">
          <Table>
            <TableHeader>
              {table.getHeaderGroups().map((hg) => (
                <TableRow key={hg.id} className="hover:bg-transparent">
                  {hg.headers.map((h) => {
                    const canSort = h.column.getCanSort()
                    return (
                      <TableHead
                        key={h.id}
                        onClick={
                          canSort
                            ? h.column.getToggleSortingHandler()
                            : undefined
                        }
                        className={canSort ? 'cursor-pointer select-none' : ''}
                      >
                        <span className="inline-flex items-center gap-1">
                          {h.isPlaceholder
                            ? null
                            : flexRender(
                                h.column.columnDef.header,
                                h.getContext(),
                              )}
                          {canSort &&
                            ({
                              asc: ' ▲',
                              desc: ' ▼',
                            }[h.column.getIsSorted() as string] ??
                              null)}
                        </span>
                      </TableHead>
                    )
                  })}
                </TableRow>
              ))}
            </TableHeader>
            <TableBody>
              {table.getRowModel().rows.map((row) => (
                <TableRow
                  key={row.id}
                  data-testid={`task-row-${row.original.sessionId}`}
                  onClick={() =>
                    navigate(
                      `/audit/${encodeURIComponent(row.original.sessionId)}`,
                    )
                  }
                  className="cursor-pointer"
                >
                  {row.getVisibleCells().map((cell) => (
                    <TableCell key={cell.id}>
                      {flexRender(
                        cell.column.columnDef.cell,
                        cell.getContext(),
                      )}
                    </TableCell>
                  ))}
                </TableRow>
              ))}
            </TableBody>
          </Table>
          {hasNextPage && (
            <div className="border-border-2 border-t bg-bg-canvas px-4 py-3 text-center">
              <Button
                variant="secondary"
                size="sm"
                onClick={fetchNextPage}
                disabled={isFetchingNextPage}
              >
                {isFetchingNextPage ? 'Loading...' : 'Load older tasks'}
              </Button>
            </div>
          )}
        </div>
      )}
    </div>
  )
}
