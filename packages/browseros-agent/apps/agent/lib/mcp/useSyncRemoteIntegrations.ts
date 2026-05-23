import { useEffect, useRef, useState } from 'react'
import { useAgentServerUrl } from '@/lib/browseros/useBrowserOSProviders'
import { type McpServer, mcpServerStorage } from './mcpServerStorage'

interface SyncState {
  isSyncing: boolean
  hasSynced: boolean
}

const fetchMCPServers = async (baseUrl: string) => {
  try {
    const response = await fetch(`${baseUrl}/klavis/servers`)
    return await response.json()
  } catch {
    return null
  }
}

/**
 * Hook to sync remote MCP integrations from Klavis proxy
 * @public
 */
export function useSyncRemoteIntegrations(): SyncState {
  const { baseUrl: agentServerUrl, isLoading: isUrlLoading } =
    useAgentServerUrl()
  const [syncState, setSyncState] = useState<SyncState>({
    isSyncing: true,
    hasSynced: false,
  })
  const hasSyncedRef = useRef(false)

  const isIntegrationsLoading = isUrlLoading

  useEffect(() => {
    const syncMissing = async () => {
      // Security Hardening & Fix: Only sync once to prevent infinite refresh loops
      // caused by state updates triggering this effect repeatedly.
      if (isIntegrationsLoading || hasSyncedRef.current || !agentServerUrl)
        return

      const serversList = await fetchMCPServers(agentServerUrl)
      if (!serversList?.servers) return

      const remoteServers = serversList.servers
      const localServers = (await mcpServerStorage.getValue()) || []

      const newServers: McpServer[] = []
      for (const remote of remoteServers) {
        if (!localServers.find((l) => l.name === remote.name)) {
          newServers.push({
            id: crypto.randomUUID(),
            name: remote.name,
            description: remote.description,
            type: 'managed',
            managedServerName: remote.name,
            managedServerDescription: remote.description,
          })
        }
      }

      if (newServers.length > 0) {
        await mcpServerStorage.setValue([...localServers, ...newServers])
      }

      hasSyncedRef.current = true
      setSyncState({ isSyncing: false, hasSynced: true })
    }

    syncMissing()
  }, [isIntegrationsLoading, agentServerUrl])

  return syncState
}
