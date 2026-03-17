import { Check, Loader2, Plus, Server, Trash2 } from 'lucide-react'
import { type FC, useState } from 'react'
import { toast } from 'sonner'
import { Button } from '@/components/ui/button'
import {
  CUSTOM_MCP_ADDED_EVENT,
  MANAGED_MCP_ADDED_EVENT,
} from '@/lib/constants/analyticsEvents'
import { useMcpServers } from '@/lib/mcp/mcpServerStorage'
import { useSyncRemoteIntegrations } from '@/lib/mcp/useSyncRemoteIntegrations'
import { track } from '@/lib/metrics/track'
import { sentry } from '@/lib/sentry/sentry'
import { AddCustomMCPDialog } from './AddCustomMCPDialog'
import { AddManagedMCPDialog } from './AddManagedMCPDialog'
import { ApiKeyDialog } from './ApiKeyDialog'
import { AvailableManagedServers } from './AvailableManagedServers'
import { McpServerIcon } from './McpServerIcon'
import { useAddManagedServer } from './useAddManagedServer'
import { useGetMCPServersList } from './useGetMCPServersList'
import { useGetUserMCPIntegrations } from './useGetUserMCPIntegrations'
import { useRemoveManagedServer } from './useRemoveManagedServer'
import { useSubmitApiKey } from './useSubmitApiKey'

const failedToAddMcp = (serverName: string, e: unknown) => {
  toast.error(`Failed to add app: ${serverName}`)
  sentry.captureException(e)
}

const failedToRemoveMcp = (serverName: string, e: unknown) => {
  toast.error(`Failed to remove app: ${serverName}`)
  sentry.captureException(e)
}

/**
 * @public
 */
export const ConnectMCP: FC = () => {
  const { servers: createdServers, addServer, removeServer } = useMcpServers()
  const [addingManagedMcp, setAddingManagedMcp] = useState(false)
  const [addingCustomMcp, setAddingCustomMcp] = useState(false)
  const [deletingServerId, setDeletingServerId] = useState<string | null>(null)
  const [apiKeyServer, setApiKeyServer] = useState<{
    name: string
    description: string
    apiKeyUrl: string
  } | null>(null)

  const { trigger: addManagedServerMutation } = useAddManagedServer()
  const { trigger: removeManagedServerMutation } = useRemoveManagedServer()
  const { trigger: submitApiKeyMutation, isMutating: isSubmittingApiKey } =
    useSubmitApiKey()

  const { data: serversList } = useGetMCPServersList()

  const {
    data: userMCPIntegrations,
    isLoading: isUserMCPIntegrationsLoading,
    mutate: mutateUserIntegrations,
  } = useGetUserMCPIntegrations()

  useSyncRemoteIntegrations()

  const openAuthUrlForMCP = async (mcpName: string) => {
    try {
      const response = await addManagedServerMutation({
        serverName: mcpName,
      })

      if (response.apiKeyUrl) {
        setApiKeyServer({
          name: mcpName,
          description: '',
          apiKeyUrl: response.apiKeyUrl,
        })
        return
      }

      if (!response.oauthUrl) {
        failedToAddMcp(mcpName, 'No auth URL returned')
        return
      }

      window.open(response.oauthUrl, '_blank')?.focus()
    } catch (e) {
      failedToAddMcp(mcpName, e)
    }
  }

  const addManagedServer = async ({
    name,
    description,
  }: {
    name: string
    description: string
  }) => {
    try {
      const response = await addManagedServerMutation({
        serverName: name,
      })

      if (!response.apiKeyUrl && !response.oauthUrl) {
        failedToAddMcp(name, 'No auth URL returned')
        return
      }

      addServer({
        id: Date.now().toString(),
        displayName: name,
        type: 'managed',
        managedServerName: name,
        managedServerDescription: description,
      })
      track(MANAGED_MCP_ADDED_EVENT, { server_name: name })

      if (response.apiKeyUrl) {
        setApiKeyServer({ name, description, apiKeyUrl: response.apiKeyUrl })
        return
      }

      window.open(response.oauthUrl, '_blank')?.focus()
    } catch (e) {
      failedToAddMcp(name, e)
    }
  }

  const handleSubmitApiKey = async (apiKey: string) => {
    if (!apiKeyServer) return
    try {
      await submitApiKeyMutation({
        serverName: apiKeyServer.name,
        apiKey,
        apiKeyUrl: apiKeyServer.apiKeyUrl,
      })
      toast.success(`${apiKeyServer.name} connected successfully`)
      setApiKeyServer(null)
      mutateUserIntegrations()
    } catch (e) {
      toast.error(
        `Failed to connect ${apiKeyServer.name}: ${e instanceof Error ? e.message : 'Unknown error'}`,
      )
      sentry.captureException(e)
    }
  }

  const deleteManagedServer = async ({
    id,
    name,
  }: {
    id: string
    name: string
  }) => {
    setDeletingServerId(id)
    try {
      const response = await removeManagedServerMutation({
        serverName: name,
      })
      if (response.success) {
        removeServer(id)
        mutateUserIntegrations()
      } else {
        failedToRemoveMcp(name, 'Success not returned from server')
      }
    } catch (e) {
      failedToRemoveMcp(name, e)
    } finally {
      setDeletingServerId(null)
    }
  }

  const addCustomServer = (config: {
    name: string
    url: string
    description: string
  }) => {
    addServer({
      id: Date.now().toString(),
      displayName: config.name,
      type: 'custom',
      config: {
        url: config.url,
        description: config.description,
      },
    })
    track(CUSTOM_MCP_ADDED_EVENT)
  }

  const availableServers = serversList?.servers.filter((eachServer) => {
    const serverName = eachServer.name
    if (
      createdServers.find((server) => server.managedServerName === serverName)
    ) {
      return false
    }
    return true
  })

  const unauthenticatedServers: { name: string; description: string }[] = []
  if (!isUserMCPIntegrationsLoading) {
    for (const server of createdServers) {
      if (server.type !== 'managed' || !server.managedServerName) continue
      const integration = userMCPIntegrations?.integrations?.find(
        (i) => i.name === server.managedServerName,
      )
      if (!integration?.is_authenticated) {
        unauthenticatedServers.push({
          name: server.managedServerName,
          description: server.managedServerDescription ?? '',
        })
      }
    }
  }

  return (
    <div className="fade-in slide-in-from-bottom-5 animate-in space-y-6 duration-500">
      {/* Header */}
      <div className="rounded-xl border border-border bg-card p-6 shadow-sm transition-all hover:shadow-md">
        <div className="flex items-start gap-4">
          <div className="flex h-12 w-12 shrink-0 items-center justify-center rounded-xl bg-[var(--accent-orange)]/10">
            <Server className="h-6 w-6 text-[var(--accent-orange)]" />
          </div>
          <div className="flex-1">
            <h2 className="mb-1 font-semibold text-xl">Connected Apps</h2>
            <p className="mb-6 text-muted-foreground text-sm">
              Connect BrowserOS assistant to apps to send email, schedule
              calendar events, write docs, and more
            </p>

            <div className="flex flex-wrap gap-3">
              <Button
                variant="outline"
                onClick={() => setAddingManagedMcp(true)}
                className="border-[var(--accent-orange)] bg-[var(--accent-orange)]/10 text-[var(--accent-orange)] hover:bg-[var(--accent-orange)]/20"
              >
                <Plus className="h-4 w-4" />
                <span>Add built-in app</span>
              </Button>

              <Button
                variant="outline"
                onClick={() => setAddingCustomMcp(true)}
              >
                <Plus className="h-4 w-4" />
                <span>Add custom app</span>
              </Button>
            </div>
          </div>
        </div>
      </div>

      {/* Created Servers */}
      {createdServers.length > 0 && (
        <div className="rounded-xl border border-border bg-card p-6 shadow-sm transition-all hover:shadow-md">
          <h3 className="mb-4 font-semibold text-lg">Your Connected Apps</h3>
          <div className="space-y-3">
            {createdServers.map((server) => (
              <div
                key={server.id}
                className="flex items-center gap-4 rounded-lg border border-border bg-background p-4 transition-all hover:border-[var(--accent-orange)]/50 hover:shadow-sm"
              >
                <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-lg bg-[var(--accent-orange)]/10">
                  <McpServerIcon
                    serverName={server.managedServerName ?? ''}
                    size={20}
                    className="text-[var(--accent-orange)]"
                  />
                </div>
                <div className="flex-1">
                  <div className="mb-1 flex items-center gap-2">
                    <span className="font-semibold">{server.displayName}</span>
                    <span
                      className={`rounded px-2 py-0.5 font-medium text-xs ${
                        server.type === 'managed'
                          ? 'bg-[var(--accent-orange)]/10 text-[var(--accent-orange)]'
                          : 'bg-muted text-muted-foreground'
                      }`}
                    >
                      {server.type === 'managed' ? 'Built-in' : 'Custom'}
                    </span>
                  </div>
                  <p className="text-muted-foreground text-sm">
                    {server.managedServerDescription ||
                      server.config?.description ||
                      server.config?.url}
                  </p>
                </div>
                {server.type === 'managed' &&
                  (isUserMCPIntegrationsLoading ? (
                    <Loader2 className="h-4 w-4 animate-spin text-muted-foreground" />
                  ) : userMCPIntegrations?.integrations?.find(
                      (i) => i.name === server.managedServerName,
                    )?.is_authenticated ? (
                    <span className="flex items-center gap-1 rounded-full bg-green-500/10 px-2 py-1 font-medium text-green-600 text-xs">
                      <Check className="h-3 w-3" />
                      Authenticated
                    </span>
                  ) : (
                    <Button
                      variant="outline"
                      size="sm"
                      onClick={() =>
                        server.managedServerName &&
                        openAuthUrlForMCP(server.managedServerName)
                      }
                    >
                      Authenticate
                    </Button>
                  ))}
                <Button
                  variant="ghost"
                  size="icon"
                  disabled={deletingServerId === server.id}
                  onClick={() => {
                    if (server.type === 'managed' && server.managedServerName) {
                      deleteManagedServer({
                        id: server.id,
                        name: server.managedServerName,
                      })
                    } else {
                      removeServer(server.id)
                    }
                  }}
                  className="text-muted-foreground hover:bg-destructive/10 hover:text-destructive"
                  title="Remove server"
                >
                  {deletingServerId === server.id ? (
                    <Loader2 className="h-4 w-4 animate-spin" />
                  ) : (
                    <Trash2 className="h-4 w-4" />
                  )}
                </Button>
              </div>
            ))}
          </div>
        </div>
      )}

      <AvailableManagedServers
        availableServers={availableServers}
        onAddServer={addManagedServer}
        isLoading={false}
      />

      <AddManagedMCPDialog
        open={addingManagedMcp}
        onOpenChange={setAddingManagedMcp}
        serversList={availableServers}
        unauthenticatedServers={unauthenticatedServers}
        onAddServer={addManagedServer}
        onAuthenticate={openAuthUrlForMCP}
      />

      <AddCustomMCPDialog
        open={addingCustomMcp}
        onOpenChange={setAddingCustomMcp}
        onAddServer={addCustomServer}
      />

      <ApiKeyDialog
        open={!!apiKeyServer}
        onOpenChange={(open) => {
          if (!open) setApiKeyServer(null)
        }}
        serverName={apiKeyServer?.name ?? ''}
        onSubmit={handleSubmitApiKey}
        isSubmitting={isSubmittingApiKey}
      />
    </div>
  )
}
