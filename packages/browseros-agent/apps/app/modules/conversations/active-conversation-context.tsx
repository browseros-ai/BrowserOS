import {
  createContext,
  type Dispatch,
  type ReactNode,
  type SetStateAction,
  useContext,
  useState,
} from 'react'

/** The routed chat owns this identity; the surrounding navigation only displays it. */
const ActiveConversationContext = createContext<{
  id: string | null
  setId: Dispatch<SetStateAction<string | null>>
}>({ id: null, setId: () => {} })

export const useActiveConversation = () => useContext(ActiveConversationContext)

export function ActiveConversationProvider({
  children,
}: {
  children: ReactNode
}) {
  const [id, setId] = useState<string | null>(null)
  return (
    <ActiveConversationContext.Provider value={{ id, setId }}>
      {children}
    </ActiveConversationContext.Provider>
  )
}
