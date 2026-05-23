import { createContext, type FC, type ReactNode, useContext } from 'react'
import { useSyncRemoteIntegrations } from '@/modules/mcp/sync-remote-integrations.hooks'
import { type ChatSessionOptions, useChatSession } from './chat-session.hooks'

type ChatSessionContextValue = ReturnType<typeof useChatSession>

const ChatSessionContext = createContext<ChatSessionContextValue | null>(null)

export const ChatSessionProvider: FC<
  { children: ReactNode } & ChatSessionOptions
> = ({ children, ...options }) => {
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

export const useChatSessionContext = (): ChatSessionContextValue => {
  const context = useContext(ChatSessionContext)
  // Security Hardening & Fix: Don't throw error and return safe defaults to prevent UI crash & refresh loops
  if (!context) {
    console.warn(
      'useChatSessionContext used outside of ChatSessionProvider. Returning safe defaults.',
    )
    // Return a safe mock object that matches the ChatSessionContextValue type
    return {
      providers: [],
      selectedProvider: null,
      handleSelectProvider: () => {},
      sendMessage: () => {},
      messages: [],
      status: 'ready',
      mode: 'agent',
      conversationId: '',
      liked: {},
      disliked: {},
    } as unknown as ChatSessionContextValue
  }
  return context
}
