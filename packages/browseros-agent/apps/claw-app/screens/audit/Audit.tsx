import { AuditEmpty } from '@/components/audit/AuditEmpty'
import { FilterBar } from '@/components/audit/FilterBar'
import { TaskRow } from '@/components/audit/TaskRow'
import { Page } from '@/components/layout/Page'
import { PageHeader } from '@/components/layout/PageHeader'
import { StatefulButton } from '@/components/motion/button/stateful'
import { Card } from '@/components/motion/card'
import { Skeleton } from '@/components/ui/skeleton'
import { HISTORY_PAGE_SIZE, useAuditScreenData } from './audit.data'

/** Everything your AI agents did with neo, newest first, one card per day. */
export function Audit() {
  const data = useAuditScreenData()
  const { filters } = data
  const now = Date.now()
  const showFilters =
    !data.isError && (data.loadedCount > 0 || data.hasActiveFilters)

  return (
    <Page>
      <PageHeader
        title="History"
        lede="Everything your AI agents did with neo, newest first. Open a task to see what happened."
      />

      {showFilters ? (
        <FilterBar
          search={filters.search}
          view={filters.view}
          agentSlug={filters.agentSlug}
          agentOptions={data.agentOptions}
          onSearchChange={data.setSearch}
          onViewChange={data.setView}
          onAgentChange={data.setAgentFilter}
        />
      ) : null}

      {data.isError ? (
        <AuditEmpty variant="error" />
      ) : data.isLoading ? (
        <DaySkeleton />
      ) : data.groups.length === 0 ? (
        data.hasActiveFilters ? (
          <AuditEmpty
            variant="search-miss"
            search={filters.search}
            onShowAll={data.clearFilters}
          />
        ) : (
          <AuditEmpty variant="zero-tasks" />
        )
      ) : (
        <div className="flex flex-col gap-9">
          {data.groups.map((group) => (
            <section
              key={group.key}
              aria-labelledby={`day-${group.key}`}
              className="flex flex-col gap-2.5"
            >
              <h2
                id={`day-${group.key}`}
                className="px-1 font-bold text-[14px] text-ink-3"
              >
                {group.label}
              </h2>
              <Card
                size="compact"
                className="gap-0 overflow-hidden border-border-2 bg-card py-0 shadow-card"
              >
                <div className="flex flex-col divide-y divide-border-2">
                  {group.tasks.map((task) => (
                    <TaskRow
                      key={task.sessionId}
                      task={task}
                      now={now}
                      timeOnly
                    />
                  ))}
                </div>
              </Card>
            </section>
          ))}
          <LoadMore
            hasNextPage={data.hasNextPage}
            isFetching={data.isFetchingNextPage}
            loadedCount={data.loadedCount}
            onLoad={data.fetchNextPage}
          />
        </div>
      )}
    </Page>
  )
}

// ---------------------------------------------------------------------------

/** How far below the viewport the next page starts loading. */
const PRELOAD_MARGIN = '600px 0px'

/**
 * Loads the next page as the end of the list nears the viewport. The button
 * stays for keyboards and for anyone who scrolls faster than the network.
 */
function LoadMore({
  hasNextPage,
  isFetching,
  loadedCount,
  onLoad,
}: {
  hasNextPage: boolean
  isFetching: boolean
  loadedCount: number
  onLoad: () => void
}) {
  if (!hasNextPage) {
    return loadedCount > HISTORY_PAGE_SIZE ? (
      <p className="self-center text-[14px] text-ink-3">
        That’s everything in your history.
      </p>
    ) : null
  }

  // The observer is DOM state; the ref callback's cleanup disconnects it.
  const watch = (node: HTMLDivElement | null) => {
    if (!node || typeof IntersectionObserver === 'undefined') return
    const observer = new IntersectionObserver(
      (entries) => {
        if (entries.some((entry) => entry.isIntersecting)) onLoad()
      },
      { rootMargin: PRELOAD_MARGIN },
    )
    observer.observe(node)
    return () => observer.disconnect()
  }

  return (
    <div ref={watch} className="flex flex-col items-center gap-3">
      {isFetching ? <RowsSkeleton count={2} /> : null}
      <StatefulButton
        variant="secondary"
        state={isFetching ? 'loading' : 'idle'}
        loadingText="Loading older tasks"
        onClick={onLoad}
        disabled={isFetching}
        className="font-semibold"
      >
        Show older tasks
      </StatefulButton>
    </div>
  )
}

function DaySkeleton() {
  return (
    <div className="flex flex-col gap-2.5">
      <Skeleton className="h-4 w-24" />
      <RowsSkeleton count={4} />
    </div>
  )
}

function RowsSkeleton({ count }: { count: number }) {
  return (
    <Card
      size="compact"
      className="w-full gap-0 overflow-hidden border-border-2 bg-card py-0 shadow-card"
    >
      <div className="flex flex-col divide-y divide-border-2">
        {Array.from({ length: count }, (_, index) => `row-${index}`).map(
          (id) => (
            <div
              key={id}
              className="grid grid-cols-[72px_minmax(0,1fr)] items-center gap-4 px-4 py-3.5 sm:grid-cols-[112px_minmax(0,1fr)] sm:px-[18px]"
            >
              <Skeleton className="aspect-[16/10] rounded-md" />
              <div className="flex flex-col gap-2">
                <Skeleton className="h-4 w-2/3" />
                <Skeleton className="h-3.5 w-11/12" />
                <Skeleton className="h-3 w-1/3" />
              </div>
            </div>
          ),
        )}
      </div>
    </Card>
  )
}
