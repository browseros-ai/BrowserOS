import { ChevronDown, Folder, Layers, PlugZap } from 'lucide-react'
import type { FC, FormEvent } from 'react'
import { useEffect, useRef, useState } from 'react'
import { AppSelector } from '@/components/elements/AppSelector'
import { WorkspaceSelector } from '@/components/elements/workspace-selector'
import { McpServerIcon } from '@/entrypoints/app/connect-mcp/McpServerIcon'
import { useGetUserMCPIntegrations } from '@/entrypoints/app/connect-mcp/useGetUserMCPIntegrations'
import { Feature } from '@/lib/browseros/capabilities'
import { useCapabilities } from '@/lib/browseros/useCapabilities'
import { useMcpServers } from '@/lib/mcp/mcpServerStorage'
import { cn } from '@/lib/utils'
import { useWorkspace } from '@/lib/workspace/use-workspace'
import { ChatAttachedTabs } from './ChatAttachedTabs'
import { ChatInput, type ChatInputHandle } from './ChatInput'
import { ChatModeToggle } from './ChatModeToggle'
import type { ChatMode } from './chatTypes'

interface ChatFooterProps {
  mode: ChatMode
  onModeChange: (mode: ChatMode) => void
  input: string
  onInputChange: (value: string) => void
  onSubmit: (e: FormEvent) => void
  status: 'streaming' | 'submitted' | 'ready' | 'error'
  onStop: () => void
  attachedTabs: chrome.tabs.Tab[]
  onToggleTab: (tab: chrome.tabs.Tab) => void
  onRemoveTab: (tabId?: number) => void
}

export const ChatFooter: FC<ChatFooterProps> = ({
  mode,
  onModeChange,
  input,
  onInputChange,
  onSubmit,
  status,
  onStop,
  attachedTabs,
  onToggleTab,
  onRemoveTab,
}) => {
  const { selectedFolder } = useWorkspace()
  const { supports } = useCapabilities()
  const { servers: mcpServers } = useMcpServers()
  const { data: userMCPIntegrations } = useGetUserMCPIntegrations()
  const chatInputRef = useRef<ChatInputHandle>(null)
  const [isTabMentionOpen, setIsTabMentionOpen] = useState(false)

  useEffect(() => {
    const focusInput = () => {
      const active = document.activeElement
      const isInteractiveElementFocused =
        active instanceof HTMLInputElement ||
        active instanceof HTMLTextAreaElement ||
        active instanceof HTMLSelectElement ||
        active instanceof HTMLButtonElement
      if (!isInteractiveElementFocused) {
        chatInputRef.current?.focus()
      }
    }

    if (document.hasFocus()) {
      focusInput()
    }

    window.addEventListener('focus', focusInput)
    return () => window.removeEventListener('focus', focusInput)
  }, [])

  const connectedManagedServers = mcpServers.filter((s) => {
    if (s.type !== 'managed' || !s.managedServerName) return false
    return userMCPIntegrations?.integrations?.find(
      (i) => i.name === s.managedServerName,
    )?.is_authenticated
  })

  return (
    <footer className="border-border/40 border-t bg-background/80 backdrop-blur-md">
      <ChatAttachedTabs tabs={attachedTabs} onRemoveTab={onRemoveTab} />

      <div className="p-3">
        <div className="flex items-center gap-2">
          <ChatModeToggle mode={mode} onModeChange={onModeChange} />

          <div className="h-4 w-px bg-border/50" />

          <div className="flex items-center gap-1">
            <button
              type="button"
              onClick={() => chatInputRef.current?.toggleTabMention()}
              data-tab-mention-trigger
              data-state={isTabMentionOpen ? 'open' : 'closed'}
              aria-expanded={isTabMentionOpen}
              aria-haspopup="dialog"
              className="flex cursor-pointer items-center gap-1 rounded-lg p-1.5 text-muted-foreground transition-colors hover:bg-muted/50 hover:text-foreground data-[state=open]:bg-accent"
              title="Attach tabs (@)"
            >
              <Layers className="h-4 w-4" />
              {attachedTabs.length > 0 && (
                <span className="font-medium text-[var(--accent-orange)] text-xs">
                  {attachedTabs.length}
                </span>
              )}
              <ChevronDown className="h-3 w-3" />
            </button>

            {supports(Feature.WORKSPACE_FOLDER_SUPPORT) && (
              <WorkspaceSelector side="top">
                <button
                  type="button"
                  className={cn(
                    'flex cursor-pointer items-center gap-1 rounded-lg p-1.5 transition-colors hover:bg-muted/50 data-[state=open]:bg-accent',
                    selectedFolder
                      ? 'text-foreground'
                      : 'text-muted-foreground hover:text-foreground',
                  )}
                  title={
                    selectedFolder
                      ? selectedFolder.name
                      : 'Select workspace folder'
                  }
                >
                  <div className="relative">
                    <Folder className="h-4 w-4" />
                    {selectedFolder && (
                      <div className="absolute -top-0.5 -right-0.5 h-1.5 w-1.5 rounded-full bg-[var(--accent-orange)]" />
                    )}
                  </div>
                  <ChevronDown className="h-3 w-3" />
                </button>
              </WorkspaceSelector>
            )}

            {supports(Feature.MANAGED_MCP_SUPPORT) && (
              <AppSelector side="top">
                <button
                  type="button"
                  className="flex cursor-pointer items-center gap-1 rounded-lg p-1.5 text-muted-foreground transition-colors hover:bg-muted/50 hover:text-foreground data-[state=open]:bg-accent"
                  title="Connect apps"
                >
                  {connectedManagedServers.length > 0 ? (
                    <>
                      <div className="flex items-center -space-x-1">
                        {connectedManagedServers.slice(0, 3).map((s) => (
                          <div
                            key={s.id}
                            className="rounded-full ring-2 ring-background"
                          >
                            <McpServerIcon
                              serverName={s.managedServerName ?? ''}
                              size={14}
                            />
                          </div>
                        ))}
                      </div>
                      {connectedManagedServers.length > 3 && (
                        <span className="font-medium text-xs">
                          +{connectedManagedServers.length - 3}
                        </span>
                      )}
                    </>
                  ) : (
                    <PlugZap className="h-4 w-4" />
                  )}
                  <ChevronDown className="h-3 w-3" />
                </button>
              </AppSelector>
            )}
          </div>
        </div>

        <ChatInput
          input={input}
          status={status}
          mode={mode}
          onInputChange={onInputChange}
          onSubmit={onSubmit}
          onStop={onStop}
          selectedTabs={attachedTabs}
          onToggleTab={onToggleTab}
          onTabMentionOpenChange={setIsTabMentionOpen}
          ref={chatInputRef}
        />
      </div>
    </footer>
  )
}
