import { useEffect, useRef, useState } from 'react'
import { useGetMCPServersList } from '@/entrypoints/app/connect-mcp/useGetMCPServersList'
import { useGetUserMCPIntegrations } from '@/entrypoints/app/connect-mcp/useGetUserMCPIntegrations'
import { type McpServer, mcpServerStorage } from './mcpServerStorage'

export interface SyncStatus {
  /** True while the initial sync is in progress (fetching + writing to storage) */
  isSyncing: boolean
  /** True once the sync has completed at least once this session */
  hasSynced: boolean
}

/**
 * Syncs remote Klavis integrations into local Chrome storage.
 *
 * Klavis ties integrations to an email address, so connecting Gmail on device A
 * and Slack on device A means device B (same email) also has Slack authenticated.
 * But local Chrome storage on device B won't know about Slack.
 *
 * This hook detects authenticated remote integrations missing from local storage
 * and adds them so they appear in the UI (and can be disconnected).
 *
 * Returns sync status so consumers can gate behavior on sync completion.
 */
export function useSyncRemoteIntegrations(): SyncStatus {
  const { data: userMCPIntegrations, isLoading: isIntegrationsLoading } =
    useGetUserMCPIntegrations()
  const { data: serversList } = useGetMCPServersList()
  const integrationsRef = useRef(userMCPIntegrations)
  const serversListRef = useRef(serversList)
  integrationsRef.current = userMCPIntegrations
  serversListRef.current = serversList
  const hasSyncedRef = useRef(false)
  const [syncState, setSyncState] = useState<SyncStatus>({
    isSyncing: true,
    hasSynced: false,
  })

  const integrationCount = userMCPIntegrations?.integrations?.length ?? 0

  useEffect(() => {
    // Still loading data — keep isSyncing: true
    if (isIntegrationsLoading) return

    // No integrations at all — nothing to sync, mark done
    if (!integrationCount) {
      setSyncState({ isSyncing: false, hasSynced: true })
      return
    }

    // Already synced this session
    if (hasSyncedRef.current) return

    const integrations = integrationsRef.current?.integrations
    if (!integrations) return

    const syncMissing = async () => {
      const localServers = await mcpServerStorage.getValue()
      const missing = integrations.filter(
        (remote) =>
          remote.is_authenticated &&
          !localServers.some((s) => s.managedServerName === remote.name),
      )

      if (missing.length > 0) {
        const catalog = serversListRef.current
        const newServers: McpServer[] = missing.map((integration) => {
          const catalogEntry = catalog?.servers.find(
            (s) => s.name === integration.name,
          )
          return {
            id: `${Date.now()}-${integration.name}`,
            displayName: integration.name,
            type: 'managed',
            managedServerName: integration.name,
            managedServerDescription: catalogEntry?.description ?? '',
          }
        })

        await mcpServerStorage.setValue([...localServers, ...newServers])
      }

      hasSyncedRef.current = true
      setSyncState({ isSyncing: false, hasSynced: true })
    }

    syncMissing()
  }, [isIntegrationsLoading, integrationCount])

  return syncState
}
