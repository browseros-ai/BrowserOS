import { useQueryClient } from '@tanstack/react-query'
import { useEffect, useState } from 'react'
import {
  type Harness,
  isUserFacingHarness,
} from '@/components/harness/harness.types'
import {
  type ConnectionState,
  useConnectHarness,
  useConnections,
  useDisconnectHarness,
} from '@/modules/api/connections.hooks'
import { resolveCanonicalMcpEndpointUrl } from '@/modules/api/mcp-endpoint'
import { toastSuccess } from '@/modules/toast/toast'

export interface ConnectedAgentsData {
  connected: ConnectionState[]
  notConnected: ConnectionState[]
  total: number
  isLoading: boolean
  isError: boolean
  endpointUrl: string | null
  pendingHarness: Harness | null
  errorFor: (harness: Harness) => string | null
  connect: (harness: Harness) => void
  disconnect: (harness: Harness) => void
}

/** Connected agents' one data hook: the agents, split by state, and the actions. */
export function useConnectedAgentsData(): ConnectedAgentsData {
  const queryClient = useQueryClient()
  const connections = useConnections({ refetchInterval: 5000 })
  const [errors, setErrors] = useState<Partial<Record<Harness, string>>>({})
  const [endpointUrl, setEndpointUrl] = useState<string | null>(null)

  // The endpoint comes from extension storage, outside React and the API.
  useEffect(() => {
    let active = true
    resolveCanonicalMcpEndpointUrl().then((resolved) => {
      if (active) setEndpointUrl(resolved)
    })
    return () => {
      active = false
    }
  }, [])

  const setError = (harness: Harness, message: string | undefined) =>
    setErrors((previous) => ({ ...previous, [harness]: message }))
  const refresh = () =>
    queryClient.invalidateQueries({ queryKey: useConnections.getKey() })

  const connectMutation = useConnectHarness({
    onMutate: ({ harness }) => setError(harness as Harness, undefined),
    onSuccess: (result, { harness }) => {
      if (result.installed) {
        toastSuccess(
          `Connected ${harness}`,
          'Restart it once so it can see neo.',
        )
      } else {
        setError(harness as Harness, result.message)
      }
      void refresh()
    },
    onError: (error, { harness }) =>
      setError(
        harness as Harness,
        error instanceof Error ? error.message : 'Couldn’t connect it.',
      ),
  })
  const disconnectMutation = useDisconnectHarness({
    onMutate: ({ harness }) => setError(harness as Harness, undefined),
    onSuccess: () => void refresh(),
    onError: (error, { harness }) =>
      setError(
        harness as Harness,
        error instanceof Error ? error.message : 'Couldn’t disconnect it.',
      ),
  })

  const visible = (connections.data?.items ?? []).filter((connection) =>
    isUserFacingHarness(connection.harness as Harness),
  )
  const pending = connectMutation.isPending
    ? connectMutation.variables?.harness
    : disconnectMutation.isPending
      ? disconnectMutation.variables?.harness
      : undefined

  return {
    connected: visible.filter((connection) => connection.installed),
    notConnected: visible.filter((connection) => !connection.installed),
    total: visible.length,
    isLoading: connections.isPending && !connections.data,
    isError: connections.isError,
    endpointUrl,
    pendingHarness: (pending as Harness | undefined) ?? null,
    errorFor: (harness) => errors[harness] ?? null,
    connect: (harness) => connectMutation.mutate({ harness }),
    disconnect: (harness) => disconnectMutation.mutate({ harness }),
  }
}
