import { createContext, type FC, type ReactNode, useContext } from 'react'
import { useSyncRemoteIntegrations } from '@/modules/mcp/sync-remote-integrations.hooks'
import { type ChatSessionOptions, useChatSession } from './chat-session.hooks'
import { usePanelConversation } from './panel-conversation.hooks'

type ChatSessionContextValue = ReturnType<typeof useChatSession>

const ChatSessionContext = createContext<ChatSessionContextValue | null>(null)

export const ChatSessionProvider: FC<
  { children: ReactNode } & ChatSessionOptions
> = (props) =>
  props.origin === 'newtab' ? (
    <SessionProvider {...props} />
  ) : (
    <PanelSessionProvider {...props} />
  )

const PanelSessionProvider: FC<
  { children: ReactNode } & ChatSessionOptions
> = ({ children, ...options }) => {
  const panel = usePanelConversation()
  if (!panel.view) return null
  return (
    <SessionProvider
      {...options}
      key={panel.view.conversationId}
      initialConversationId={panel.view.conversationId}
      panelTabId={panel.tabId}
      panelRun={panel.view.runId}
      onSelectConversation={panel.select}
    >
      {children}
    </SessionProvider>
  )
}

// A new selection owns a fresh SDK instance and callbacks. Hidden native panel
// documents stay mounted, but cannot mutate the chat selected in another tab.
const SessionProvider: FC<{ children: ReactNode } & ChatSessionOptions> = ({
  children,
  ...options
}) => {
  const { hasSynced } = useSyncRemoteIntegrations()
  const session = useChatSession({
    ...options,
    isIntegrationsSynced: hasSynced,
  })
  return (
    <ChatSessionContext.Provider value={session}>
      {children}
    </ChatSessionContext.Provider>
  )
}

export const useChatSessionContext = () => {
  const context = useContext(ChatSessionContext)
  if (!context) {
    throw new Error(
      'useChatSessionContext must be used within a ChatSessionProvider',
    )
  }
  return context
}
