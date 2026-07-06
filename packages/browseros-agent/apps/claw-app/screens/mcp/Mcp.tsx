import { useQueryClient } from '@tanstack/react-query'
import { useEffect, useMemo, useState } from 'react'
import {
  type Harness,
  RETIRED_HARNESSES,
} from '@/components/harness/harness.types'
import { EditorialEmpty } from '@/components/ui/EditorialEmpty'
import {
  useBrowserosConnections,
  useConnectBrowseros,
  useDisconnectBrowseros,
} from '@/modules/api/connections.hooks'
import { resolveCanonicalMcpEndpointUrl } from '@/modules/api/mcp-endpoint'
import { ConnectionRow } from './ConnectionRow'
import { HeroCard } from './HeroCard'

/** Harnesses that should not show a user-facing Connect flow. */
const HIDDEN_HARNESSES: readonly Harness[] = [
  ...RETIRED_HARNESSES,
  'Hermes',
  'OpenClaw',
  'Gemini CLI',
]

export function Mcp() {
  const [url, setUrl] = useState<string | null>(null)
  const connections = useBrowserosConnections()
  const connect = useConnectBrowseros()
  const disconnect = useDisconnectBrowseros()
  const queryClient = useQueryClient()
  const [errors, setErrors] = useState<Partial<Record<Harness, string>>>({})

  useEffect(() => {
    let active = true
    resolveCanonicalMcpEndpointUrl().then((resolved) => {
      if (active) setUrl(resolved)
    })
    return () => {
      active = false
    }
  }, [])

  const isLoading = connections.isPending && !connections.data

  const visibleRows = useMemo(() => {
    const list = connections.data?.connections ?? []
    return list.filter((c) => !HIDDEN_HARNESSES.includes(c.harness))
  }, [connections.data])

  const connectedCount = visibleRows.filter((c) => c.installed).length
  const totalCount = visibleRows.length

  const onConnect = async (harness: Harness) => {
    setErrors((prev) => ({ ...prev, [harness]: undefined }))
    try {
      const result = await connect.mutateAsync({ harness })
      if (!result.installed) {
        setErrors((prev) => ({ ...prev, [harness]: result.message }))
      }
      void queryClient.invalidateQueries({
        queryKey: useBrowserosConnections.getKey(),
      })
    } catch (err) {
      setErrors((prev) => ({
        ...prev,
        [harness]: err instanceof Error ? err.message : 'Failed to connect.',
      }))
    }
  }

  const onDisconnect = async (harness: Harness) => {
    setErrors((prev) => ({ ...prev, [harness]: undefined }))
    try {
      await disconnect.mutateAsync({ harness })
      void queryClient.invalidateQueries({
        queryKey: useBrowserosConnections.getKey(),
      })
    } catch (err) {
      setErrors((prev) => ({
        ...prev,
        [harness]: err instanceof Error ? err.message : 'Failed to disconnect.',
      }))
    }
  }

  const pendingHarness =
    connect.isPending && connect.variables
      ? connect.variables.harness
      : disconnect.isPending && disconnect.variables
        ? disconnect.variables.harness
        : null

  return (
    <div className="mx-auto flex w-full max-w-xl flex-col gap-8 px-8 pt-8 pb-16">
      <HeroCard url={url} />
      <section className="space-y-2">
        <header className="flex items-baseline justify-between gap-3">
          <h2 className="font-semibold text-ink text-lg">Connected agents</h2>
          {!isLoading && !connections.isError && (
            <span className="font-mono text-[10.5px] text-ink-3 uppercase tabular-nums tracking-[0.08em]">
              {connectedCount} of {totalCount} connected
            </span>
          )}
        </header>
        {isLoading ? (
          <SkeletonList />
        ) : connections.isError ? (
          <EditorialEmpty
            leading="could not"
            accent="reach"
            trailing="the cockpit."
            hint="Check that the local claw-server is running."
          />
        ) : (
          <div>
            {visibleRows.map((state) => (
              <ConnectionRow
                key={state.harness}
                state={state}
                isPending={pendingHarness === state.harness}
                errorMessage={errors[state.harness] ?? null}
                onConnect={() => onConnect(state.harness)}
                onDisconnect={() => onDisconnect(state.harness)}
              />
            ))}
          </div>
        )}
      </section>
    </div>
  )
}

/**
 * 6 hairline skeleton rows shaped like the real ConnectionRow so
 * the layout does not jump when the connections query resolves.
 */
function SkeletonList() {
  return (
    <div>
      {['s1', 's2', 's3', 's4', 's5', 's6'].map((id) => (
        <div key={id} className="border-border-2 border-t">
          <div className="flex items-center gap-3 py-3">
            <div className="size-5 shrink-0 animate-pulse rounded bg-card-tint" />
            <div className="flex-1">
              <div className="h-3 w-32 animate-pulse rounded bg-card-tint" />
            </div>
            <div className="h-3 w-16 animate-pulse rounded bg-card-tint" />
          </div>
        </div>
      ))}
    </div>
  )
}
