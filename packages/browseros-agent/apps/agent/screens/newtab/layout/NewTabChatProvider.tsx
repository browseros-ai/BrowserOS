import {
  createContext,
  type FC,
  type ReactNode,
  useContext,
  useState,
} from 'react'
import type { Provider } from '@/components/chat/chatComponentTypes'
import type { ChatMode } from '@/entrypoints/sidepanel/index/chatTypes'

interface NewTabChatContextValue {
  providers: Provider[]
  selectedProvider: Provider | null
  handleSelectProvider: (provider: Provider) => void
  mode: ChatMode
  setMode: (mode: ChatMode) => void
}

const NewTabChatContext = createContext<NewTabChatContextValue | null>(null)

export const NewTabChatProvider: FC<{ children: ReactNode }> = ({
  children,
}) => {
  const [providers] = useState<Provider[]>([])
  const [selectedProvider, setSelectedProvider] = useState<Provider | null>(
    null,
  )
  const [mode, setMode] = useState<ChatMode>('agent')

  const handleSelectProvider = (provider: Provider) => {
    setSelectedProvider(provider)
  }

  return (
    <NewTabChatContext.Provider
      value={{
        providers,
        selectedProvider,
        handleSelectProvider,
        mode,
        setMode,
      }}
    >
      {children}
    </NewTabChatContext.Provider>
  )
}

export const useNewTabChatContext = () => {
  const context = useContext(NewTabChatContext)
  if (!context) {
    throw new Error(
      'useNewTabChatContext must be used within a NewTabChatProvider',
    )
  }
  return context
}
