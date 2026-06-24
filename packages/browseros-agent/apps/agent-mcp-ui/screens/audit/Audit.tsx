import { ScrollText } from 'lucide-react'
import { AgentFilterPills } from '@/components/audit/AgentFilterPills'
import { DispatchRow } from '@/components/audit/DispatchRow'
import { EmptyState } from '@/components/cockpit/EmptyState'
import { Spinner } from '@/components/ui/spinner'
import { useAuditScreenData } from './audit.data'

/**
 * v2 audit log screen. Streams every persisted tool dispatch from
 * `<browserosDir>/mcp-interface/audit.sqlite` via the `useDispatches`
 * infinite query. Filter pills above the list narrow to one agent;
 * each row click reveals the args + result meta. The list updates
 * every 3 seconds via the hook's refetchInterval.
 */
export function Audit() {
  const {
    rows,
    chips,
    isLoading,
    isError,
    hasNextPage,
    isFetchingNextPage,
    fetchNextPage,
    selectedAgentId,
    setSelectedAgentId,
    now,
  } = useAuditScreenData()

  return (
    <div className="mx-auto flex w-full max-w-5xl flex-col gap-6 px-8 pt-10 pb-20">
      <header className="space-y-2">
        <div className="flex items-center gap-2.5">
          <span className="flex size-9 items-center justify-center rounded-xl bg-accent-tint text-accent">
            <ScrollText className="size-4.5" />
          </span>
          <div>
            <h1 className="font-extrabold text-2xl tracking-tight">Audit</h1>
            <p className="text-ink-3 text-sm">
              Every successful tool dispatch persisted to local SQLite. Filter
              by agent, expand a row for arguments and result.
            </p>
          </div>
        </div>
      </header>

      {!isLoading && !isError && rows.length > 0 && (
        <AgentFilterPills
          chips={chips}
          selectedAgentId={selectedAgentId}
          onSelect={setSelectedAgentId}
        />
      )}

      {isLoading ? (
        <div className="flex justify-center py-12 text-ink-3">
          <Spinner />
        </div>
      ) : isError ? (
        <EmptyState
          title="Could not load audit log"
          hint="Check that the cockpit server is running and the audit database is reachable."
        />
      ) : rows.length === 0 ? (
        <EmptyState
          title="No dispatches yet"
          hint="Connect an agent via the MCP page and run a tool. Successful dispatches land here within a few seconds."
        />
      ) : (
        <section className="overflow-hidden rounded-2xl border border-border-2 bg-card">
          {rows.map((row) => (
            <DispatchRow key={row.id} row={row} now={now} />
          ))}
          {hasNextPage && (
            <div className="border-border-2 border-t bg-bg-canvas px-4 py-3 text-center">
              <button
                type="button"
                onClick={fetchNextPage}
                disabled={isFetchingNextPage}
                className="inline-flex items-center gap-2 rounded-md bg-bg-sunken px-3 py-1.5 font-semibold text-[12.5px] text-ink-2 transition hover:bg-card-tint disabled:cursor-not-allowed disabled:opacity-50"
              >
                {isFetchingNextPage ? 'Loading...' : 'Load older dispatches'}
              </button>
            </div>
          )}
        </section>
      )}
    </div>
  )
}
