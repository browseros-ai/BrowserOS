import { useQueryClient } from '@tanstack/react-query'
import { useState } from 'react'
import { Spinner } from '@/components/ui/spinner'
import {
  useBrowserosConnections,
  useConnectBrowseros,
  useDisconnectBrowseros,
} from '@/modules/api/connections.hooks'
import {
  buildCanonicalMcpCliCommand,
  buildCanonicalMcpEndpointUrl,
} from '@/modules/api/mcp-endpoint'
import type { Harness } from '@/screens/new-agent/new-agent.schemas'
import { ConnectionRow } from './ConnectionRow'
import { HeroCard } from './HeroCard'

/**
 * v2 MCP page. Hero card with the single canonical endpoint plus a
 * per-harness "Connect" board that drives `agent-mcp-manager` so the
 * user installs BrowserOS into Claude Code / Cursor / VS Code / Codex
 * with one click. Hermes and OpenClaw are BrowserOS-internal and
 * render as "Built-in" rows.
 *
 * Live MCP-session state (who is connected right now) is surfaced on
 * the homepage's running grid, not here; this page is the install
 * board.
 */
export function Mcp() {
  const url = buildCanonicalMcpEndpointUrl()
  const cli = buildCanonicalMcpCliCommand()
  const connections = useBrowserosConnections()
  const connect = useConnectBrowseros()
  const disconnect = useDisconnectBrowseros()
  const queryClient = useQueryClient()
  const [errors, setErrors] = useState<Partial<Record<Harness, string>>>({})

  const isLoading = connections.isPending && !connections.data
  const list = connections.data?.connections ?? []
  const externalRows = list.filter((c) => c.agentId !== null)
  const externalConnected = externalRows.filter((c) => c.installed).length
  const externalTotal = externalRows.length

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
    <div className="mx-auto flex w-full max-w-3xl flex-col gap-6 px-8 pt-6 pb-20">
      <HeroCard url={url} cli={cli} />
      <section className="rounded-2xl border border-border-2 bg-card">
        <div className="flex items-start gap-3 border-border-2 border-b px-4 py-4">
          <div className="flex-1">
            <h2 className="font-bold text-base">Connected agents</h2>
            <p className="mt-0.5 text-ink-3 text-sm">
              Add BrowserOS as an MCP server in your AI agents. No copy-paste
              required.
            </p>
          </div>
          {!isLoading && (
            <span className="inline-flex shrink-0 items-center gap-1 rounded-full bg-bg-sunken px-3 py-1 font-bold text-[12px] text-ink-2">
              {externalConnected} of {externalTotal} connected
            </span>
          )}
        </div>
        {isLoading ? (
          <div className="flex justify-center py-10 text-ink-3">
            <Spinner />
          </div>
        ) : connections.isError ? (
          <div className="px-4 py-6 text-center text-ink-3 text-sm">
            Could not load the connection list. Check that the cockpit server is
            running.
          </div>
        ) : (
          <div className="flex flex-col">
            {list.map((state) => (
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
      <p className="text-[12px] text-ink-3 leading-relaxed">
        Hermes and OpenClaw run inside BrowserOS and don't need a config write.
      </p>
    </div>
  )
}
