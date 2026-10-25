import { TriangleAlert } from 'lucide-react'
import type { ReactNode } from 'react'
import type { Harness } from '@/components/harness/harness.types'
import { Page } from '@/components/layout/Page'
import { PageHeader } from '@/components/layout/PageHeader'
import { Card } from '@/components/motion/card'
import { Skeleton } from '@/components/ui/skeleton'
import type { ConnectionState } from '@/modules/api/connections.hooks'
import { ClaudeDesktopCard } from './ClaudeDesktopCard'
import { ConnectionRow } from './ConnectionRow'
import { ManualSetupCard } from './ManualSetupCard'
import { type ConnectedAgentsData, useConnectedAgentsData } from './mcp.data'

/** Which AI agents can use neo, and the one step to connect each. */
export function Mcp() {
  const data = useConnectedAgentsData()

  return (
    <Page>
      <PageHeader
        title="Connected agents"
        lede="These AI agents can use neo to browse for you. Connect one, then restart it once so it can see neo."
      />

      <RowsCard>
        <ClaudeDesktopCard />
      </RowsCard>

      {data.isError ? (
        <Card
          size="compact"
          className="flex-row items-center gap-4 border-border-2 bg-card px-[22px] py-5 shadow-card"
        >
          <span
            aria-hidden
            className="grid size-11 shrink-0 place-items-center rounded-full bg-red-tint text-red-ink"
          >
            <TriangleAlert className="size-5" />
          </span>
          <span className="flex min-w-0 flex-col gap-0.5">
            <span className="font-bold text-[16px] text-cyanotype-ink">
              Couldn’t check your agents
            </span>
            <span className="text-[14.5px] text-ink-2">
              Check that BrowserOS is running, then open this page again.
            </span>
          </span>
        </Card>
      ) : data.isLoading ? (
        <Group title="Connected">
          <RowsSkeleton />
        </Group>
      ) : (
        <>
          <Group
            title="Connected"
            count={`${data.connected.length} of ${data.total}`}
          >
            {data.connected.length ? (
              <AgentRows rows={data.connected} data={data} />
            ) : (
              <p className="px-[18px] py-4 text-[14.5px] text-ink-2">
                No agents connected yet. Pick one below.
              </p>
            )}
          </Group>
          {data.notConnected.length ? (
            <Group title="Not connected yet">
              <AgentRows rows={data.notConnected} data={data} />
            </Group>
          ) : null}
        </>
      )}

      <ManualSetupCard endpointUrl={data.endpointUrl} />
    </Page>
  )
}

// ---------------------------------------------------------------------------

function Group({
  title,
  count,
  children,
}: {
  title: string
  count?: string
  children: ReactNode
}) {
  return (
    <section aria-label={title} className="flex flex-col gap-3.5">
      <div className="flex items-baseline justify-between gap-4">
        <h2 className="font-bold text-[19px] text-cyanotype-ink tracking-[-0.015em]">
          {title}
        </h2>
        {count ? (
          <span className="text-[13.5px] text-ink-3 tabular-nums">{count}</span>
        ) : null}
      </div>
      <RowsCard>{children}</RowsCard>
    </section>
  )
}

function RowsCard({ children }: { children: ReactNode }) {
  return (
    <Card
      size="compact"
      className="gap-0 overflow-hidden border-border-2 bg-card py-0 shadow-card"
    >
      <ul className="flex flex-col divide-y divide-border-2">{children}</ul>
    </Card>
  )
}

function AgentRows({
  rows,
  data,
}: {
  rows: ConnectionState[]
  data: ConnectedAgentsData
}) {
  return rows.map((state) => {
    const harness = state.harness as Harness
    return (
      <ConnectionRow
        key={harness}
        state={state}
        isPending={data.pendingHarness === harness}
        errorMessage={data.errorFor(harness)}
        onConnect={() => data.connect(harness)}
        onDisconnect={() => data.disconnect(harness)}
      />
    )
  })
}

function RowsSkeleton() {
  return ['s1', 's2', 's3'].map((id) => (
    <li key={id} className="flex items-center gap-4 px-[18px] py-4">
      <Skeleton className="size-11 rounded-xl" />
      <div className="flex flex-1 flex-col gap-2">
        <Skeleton className="h-4 w-32" />
        <Skeleton className="h-3.5 w-24" />
      </div>
      <Skeleton className="h-8 w-24 rounded-full" />
    </li>
  ))
}
