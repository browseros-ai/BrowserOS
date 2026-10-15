import type { OAuthStatus } from '@browseros/server-api'
import { ServerApiError } from '@browseros/server-api-client'
import { useEffect, useRef, useState } from 'react'
import { getAgentServerUrl } from '@/lib/browseros/helpers'
import { serverApi, serverData } from '@/lib/server-api/client'

export type { OAuthStatus } from '@browseros/server-api'

export interface UseOAuthStatusReturn {
  status: OAuthStatus | null
  isPolling: boolean
  startPolling: () => void
  stopPolling: () => void
  refresh: () => Promise<OAuthStatus | null>
  disconnect: () => Promise<void>
}

export function useOAuthStatus(provider: string): UseOAuthStatusReturn {
  const [status, setStatus] = useState<OAuthStatus | null>(null)
  const [isPolling, setIsPolling] = useState(false)
  const pollIntervalRef = useRef<ReturnType<typeof setInterval> | null>(null)
  const pollTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null)

  async function fetchStatus(): Promise<OAuthStatus | null> {
    try {
      const serverUrl = await getAgentServerUrl()
      const data = await serverData(
        serverApi(serverUrl).http.GET('/oauth/{provider}/status', {
          params: { path: { provider } },
        }),
      )
      setStatus(data)
      return data
    } catch {
      return null
    }
  }

  function stopPolling() {
    if (pollIntervalRef.current) clearInterval(pollIntervalRef.current)
    if (pollTimeoutRef.current) clearTimeout(pollTimeoutRef.current)
    pollIntervalRef.current = null
    pollTimeoutRef.current = null
    setIsPolling(false)
  }

  function startPolling() {
    stopPolling()
    setIsPolling(true)

    pollIntervalRef.current = setInterval(async () => {
      const result = await fetchStatus()
      if (result?.authenticated) {
        stopPolling()
      }
    }, 2_000)

    pollTimeoutRef.current = setTimeout(stopPolling, 300_000)
  }

  async function disconnect() {
    try {
      const serverUrl = await getAgentServerUrl()
      await serverApi(serverUrl)
        .http.DELETE('/oauth/{provider}', {
          params: { path: { provider } },
        })
        .catch((error: unknown) => {
          // Disconnect is best-effort: any HTTP response clears the local status,
          // matching the existing UI while network failures leave it unchanged.
          if (!(error instanceof ServerApiError)) throw error
        })
      setStatus({ authenticated: false, provider })
    } catch {
      // Best-effort disconnect
    }
  }

  // Initial status check on mount
  // biome-ignore lint/correctness/useExhaustiveDependencies: only run on mount
  useEffect(() => {
    fetchStatus()
  }, [])

  // Cleanup on unmount
  // biome-ignore lint/correctness/useExhaustiveDependencies: cleanup only needs to run on unmount
  useEffect(() => {
    return () => stopPolling()
  }, [])

  return {
    status,
    isPolling,
    startPolling,
    stopPolling,
    refresh: fetchStatus,
    disconnect,
  }
}
