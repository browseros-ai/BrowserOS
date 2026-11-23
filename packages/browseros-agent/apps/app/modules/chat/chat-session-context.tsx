import { createContext, type FC, type ReactNode, useContext } from 'react'
import { type ChatSessionOptions, useChatSession } from './chat-session.hooks'

type ChatSessionContextValue = ReturnType<typeof useChatSession>

const ChatSessionContext = createContext<ChatSessionContextValue | null>(null)

export const ChatSessionProvider: FC<
  { children: ReactNode } & ChatSessionOptions
> = ({ children, ...options }) => {
  const session = useChatSession(options)
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
