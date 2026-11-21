import { ServerApiClient } from '@browseros/server-api/client'
import { useEffect, useRef, useState } from 'react'
import { getAgentServerUrl } from '@/lib/browseros/helpers'

export interface OAuthStatus {
  authenticated: boolean
  email?: string
  provider: string
}

export interface UseOAuthStatusReturn {
  status: OAuthStatus | null
  isPolling: boolean
  startPolling: () => void
  stopPolling: () => void
  refresh: () => Promise<OAuthStatus | null>
  disconnect: () => Promise<void>
}

export function useOAuthStatus(provider: 'chatgpt-pro'): UseOAuthStatusReturn {
  const [status, setStatus] = useState<OAuthStatus | null>(null)
  const [isPolling, setIsPolling] = useState(false)
  const pollIntervalRef = useRef<ReturnType<typeof setInterval> | null>(null)
  const pollTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null)

  const statusVersion = useRef(0)

  async function fetchStatus(): Promise<OAuthStatus | null> {
    const version = statusVersion.current
    try {
      const serverUrl = await getAgentServerUrl()
      const result = await new ServerApiClient(serverUrl).getSignInStatus({
        provider,
      })
      // An earlier poll can resolve after sign-out. Its snapshot is stale.
      if (version !== statusVersion.current) return null
      const data = { ...result, email: result.email ?? undefined }
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
      const { success } = await new ServerApiClient(serverUrl).signOut({
        provider,
      })
      if (!success) return
      statusVersion.current += 1
      stopPolling()
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
