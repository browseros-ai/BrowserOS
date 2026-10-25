import { AuditEmpty } from '@/components/audit/AuditEmpty'
import { FilterBar } from '@/components/audit/FilterBar'
import { ManageAuditFilesDialog } from '@/components/audit/ManageAuditFilesDialog'
import { TaskRow } from '@/components/audit/TaskRow'
import { Page } from '@/components/layout/Page'
import { PageHeader } from '@/components/layout/PageHeader'
import { StatefulButton } from '@/components/motion/button/stateful'
import { Card } from '@/components/motion/card'
import { Skeleton } from '@/components/ui/skeleton'
import { useAuditScreenData } from './audit.data'

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
        actions={<ManageAuditFilesDialog />}
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
          {data.hasNextPage ? (
            <StatefulButton
              variant="secondary"
              state={data.isFetchingNextPage ? 'loading' : 'idle'}
              loadingText="Loading older tasks"
              onClick={data.fetchNextPage}
              disabled={data.isFetchingNextPage}
              className="self-center font-semibold"
            >
              Show older tasks
            </StatefulButton>
          ) : null}
        </div>
      )}
    </Page>
  )
}

// ---------------------------------------------------------------------------

function DaySkeleton() {
  return (
    <div className="flex flex-col gap-2.5">
      <Skeleton className="h-4 w-24" />
      <Card
        size="compact"
        className="gap-0 overflow-hidden border-border-2 bg-card py-0 shadow-card"
      >
        <div className="flex flex-col divide-y divide-border-2">
          {['r1', 'r2', 'r3', 'r4'].map((id) => (
            <div
              key={id}
              className="grid grid-cols-[72px_minmax(0,1fr)] items-center gap-4 px-4 py-3.5 sm:grid-cols-[112px_minmax(0,1fr)] sm:px-[18px]"
            >
              <Skeleton className="aspect-[16/10] rounded-[10px]" />
              <div className="flex flex-col gap-2">
                <Skeleton className="h-4 w-2/3" />
                <Skeleton className="h-3.5 w-11/12" />
                <Skeleton className="h-3 w-1/3" />
              </div>
            </div>
          ))}
        </div>
      </Card>
    </div>
  )
}
