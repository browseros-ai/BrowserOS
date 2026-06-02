import { KeyRound, Plus, Settings } from 'lucide-react'
import type { FC, ReactNode } from 'react'
import { useState } from 'react'
import { toast } from 'sonner'
import {
  Command,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
} from '@/components/ui/command'
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from '@/components/ui/popover'
import { ApiKeyDialog } from '@/entrypoints/app/connect-mcp/ApiKeyDialog'
import { McpServerIcon } from '@/entrypoints/app/connect-mcp/McpServerIcon'
import { useAddManagedServer } from '@/entrypoints/app/connect-mcp/useAddManagedServer'
import { useGetMCPServersList } from '@/entrypoints/app/connect-mcp/useGetMCPServersList'
import { useGetUserMCPIntegrations } from '@/entrypoints/app/connect-mcp/useGetUserMCPIntegrations'
import { useSubmitApiKey } from '@/entrypoints/app/connect-mcp/useSubmitApiKey'
import { MANAGED_MCP_ADDED_EVENT } from '@/lib/constants/analyticsEvents'
import { useMcpServers } from '@/lib/mcp/mcpServerStorage'
import { useSyncRemoteIntegrations } from '@/lib/mcp/useSyncRemoteIntegrations'
import { track } from '@/lib/metrics/track'
import { sentry } from '@/lib/sentry/sentry'

interface AppSelectorProps {
  children: ReactNode
  side?: 'top' | 'bottom' | 'left' | 'right'
}

export const AppSelector: FC<AppSelectorProps> = ({
  children,
  side = 'bottom',
}) => {
  const [open, setOpen] = useState(false)
  const [filterText, setFilterText] = useState('')
  const [apiKeyServer, setApiKeyServer] = useState<{
    name: string
    apiKeyUrl: string
  } | null>(null)

  const { servers: createdServers, addServer } = useMcpServers()
  useSyncRemoteIntegrations()
  const { trigger: addManagedServerMutation } = useAddManagedServer()
  const { trigger: submitApiKeyMutation, isMutating: isSubmittingApiKey } =
    useSubmitApiKey()
  const { data: serversList } = useGetMCPServersList()
  const {
    data: userMCPIntegrations,
    isLoading: isIntegrationsLoading,
    mutate: mutateUserIntegrations,
  } = useGetUserMCPIntegrations()

  const query = filterText.toLowerCase()

  const connectedServers = createdServers.filter((s) => {
    if (s.type !== 'managed' || !s.managedServerName) return false
    const integration = userMCPIntegrations?.integrations?.find(
      (i) => i.name === s.managedServerName,
    )
    return integration?.is_authenticated === true
  })

  const unauthenticatedServers = createdServers.filter((s) => {
    if (s.type !== 'managed' || !s.managedServerName) return false
    if (isIntegrationsLoading) return false
    const integration = userMCPIntegrations?.integrations?.find(
      (i) => i.name === s.managedServerName,
    )
    return !integration?.is_authenticated
  })

  const availableServers =
    serversList?.servers.filter((s) => {
      return !createdServers.find(
        (created) => created.managedServerName === s.name,
      )
    }) ?? []

  const filteredConnected = connectedServers.filter(
    (s) =>
      s.displayName.toLowerCase().includes(query) ||
      s.managedServerDescription?.toLowerCase().includes(query),
  )

  const filteredUnauthenticated = unauthenticatedServers.filter(
    (s) =>
      s.displayName.toLowerCase().includes(query) ||
      s.managedServerDescription?.toLowerCase().includes(query),
  )

  const filteredAvailable = availableServers.filter(
    (s) =>
      s.name.toLowerCase().includes(query) ||
      s.description.toLowerCase().includes(query),
  )

  const hasResults =
    filteredConnected.length > 0 ||
    filteredUnauthenticated.length > 0 ||
    filteredAvailable.length > 0

  const openAuthUrl = async (serverName: string) => {
    try {
      const response = await addManagedServerMutation({ serverName })
      if (response.apiKeyUrl) {
        setApiKeyServer({ name: serverName, apiKeyUrl: response.apiKeyUrl })
        return
      }
      if (!response.oauthUrl) {
        toast.error(`Failed to add app: ${serverName}`)
        return
      }
      window.open(response.oauthUrl, '_blank')?.focus()
    } catch (e) {
      toast.error(`Failed to add app: ${serverName}`)
      sentry.captureException(e)
    }
  }

  const handleAddServer = async (name: string, description: string) => {
    try {
      const response = await addManagedServerMutation({ serverName: name })
      addServer({
        id: Date.now().toString(),
        displayName: name,
        type: 'managed',
        managedServerName: name,
        managedServerDescription: description,
      })
      track(MANAGED_MCP_ADDED_EVENT, { server_name: name })

      if (response.apiKeyUrl) {
        setApiKeyServer({ name, apiKeyUrl: response.apiKeyUrl })
        return
      }
      if (!response.oauthUrl) {
        toast.error(`Failed to add app: ${name}`)
        return
      }
      window.open(response.oauthUrl, '_blank')?.focus()
    } catch (e) {
      toast.error(`Failed to add app: ${name}`)
      sentry.captureException(e)
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

  return (
    <>
      <Popover open={open} onOpenChange={setOpen}>
        <PopoverTrigger asChild>{children}</PopoverTrigger>
        <PopoverContent
          side={side}
          align="end"
          className="w-72 p-0"
          role="dialog"
          aria-label="Connect apps"
        >
          <Command
            className="[&_svg:not([class*='text-'])]:text-muted-foreground"
            shouldFilter={false}
          >
            <CommandInput
              placeholder="Search apps..."
              className="h-9"
              value={filterText}
              onValueChange={setFilterText}
            />
            <CommandList className="max-h-64 overflow-auto">
              <CommandEmpty>No apps found</CommandEmpty>

              {filteredConnected.length > 0 && (
                <CommandGroup>
                  <div className="my-2 px-2 font-semibold text-muted-foreground text-xs uppercase tracking-wide">
                    Connected
                  </div>
                  <div className="flex flex-wrap items-center gap-2 px-3 py-2">
                    {filteredConnected.map((server) => (
                      <div
                        key={server.id}
                        title={server.displayName}
                        className="flex h-8 w-8 items-center justify-center rounded-lg border border-border bg-accent/50"
                      >
                        <McpServerIcon
                          serverName={server.managedServerName ?? ''}
                          size={18}
                        />
                      </div>
                    ))}
                    <button
                      type="button"
                      onClick={() => {
                        const appUrl = chrome.runtime.getURL(
                          '/app.html#/connect-apps',
                        )
                        window.open(appUrl, '_blank')
                        setOpen(false)
                      }}
                      title="Manage apps"
                      className="flex h-8 w-8 cursor-pointer items-center justify-center rounded-lg border border-border border-dashed text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
                    >
                      <Settings className="h-3.5 w-3.5" />
                    </button>
                  </div>
                </CommandGroup>
              )}

              {filteredUnauthenticated.length > 0 && (
                <CommandGroup>
                  <div className="my-2 px-2 font-semibold text-muted-foreground text-xs uppercase tracking-wide">
                    Needs authentication
                  </div>
                  {filteredUnauthenticated.map((server) => (
                    <CommandItem
                      key={server.id}
                      value={`${server.id} ${server.displayName}`}
                      onSelect={() =>
                        server.managedServerName &&
                        openAuthUrl(server.managedServerName)
                      }
                      className="flex cursor-pointer items-center gap-3 px-3 py-2"
                    >
                      <McpServerIcon
                        serverName={server.managedServerName ?? ''}
                        size={18}
                        className="shrink-0"
                      />
                      <span className="flex-1 truncate text-sm">
                        {server.displayName}
                      </span>
                      <KeyRound className="h-3.5 w-3.5 shrink-0 text-amber-500" />
                    </CommandItem>
                  ))}
                </CommandGroup>
              )}

              {filteredAvailable.length > 0 && (
                <CommandGroup>
                  <div className="my-2 px-2 font-semibold text-muted-foreground text-xs uppercase tracking-wide">
                    Available
                  </div>
                  {filteredAvailable.map((server) => (
                    <CommandItem
                      key={server.name}
                      value={`${server.name} ${server.description}`}
                      onSelect={() =>
                        handleAddServer(server.name, server.description)
                      }
                      className="flex cursor-pointer items-center gap-3 px-3 py-2"
                    >
                      <McpServerIcon
                        serverName={server.name}
                        size={18}
                        className="shrink-0"
                      />
                      <div className="min-w-0 flex-1">
                        <span className="block truncate text-sm">
                          {server.name}
                        </span>
                        {server.description && (
                          <span className="block truncate text-muted-foreground text-xs">
                            {server.description}
                          </span>
                        )}
                      </div>
                      <Plus className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
                    </CommandItem>
                  ))}
                </CommandGroup>
              )}

              {!hasResults && filterText && null}
            </CommandList>
          </Command>
        </PopoverContent>
      </Popover>

      <ApiKeyDialog
        open={!!apiKeyServer}
        onOpenChange={(isOpen) => {
          if (!isOpen) setApiKeyServer(null)
        }}
        serverName={apiKeyServer?.name ?? ''}
        onSubmit={handleSubmitApiKey}
        isSubmitting={isSubmittingApiKey}
      />
    </>
  )
}
