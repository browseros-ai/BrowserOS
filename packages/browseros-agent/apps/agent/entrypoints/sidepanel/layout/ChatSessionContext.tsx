import { createContext, type FC, type ReactNode, useContext } from 'react'
import { useSyncRemoteIntegrations } from '@/lib/mcp/useSyncRemoteIntegrations'
import {
  type ChatSessionOptions,
  useChatSession,
} from '../index/useChatSession'

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

export const useChatSessionContext = () => {
  const context = useContext(ChatSessionContext)
  if (!context) {
    throw new Error(
      'useChatSessionContext must be used within a ChatSessionProvider',
    )
  }
  return context
}
