import type { FC } from 'react'
import { Outlet, useLocation } from 'react-router'
import { ChatSessionProvider } from '@/modules/chat/chat-session-context'
import { NewTabFocusGrid } from './NewTabFocusGrid'
import { shouldHideFocusGrid, shouldUseChatSession } from './route-utils'

export const NewTabLayout: FC = () => {
  const location = useLocation()
  const hideGrid = shouldHideFocusGrid(location.pathname)
  const useChatSession = shouldUseChatSession(location.pathname)
  const content = (
    <>
      {!hideGrid && <NewTabFocusGrid />}
      <Outlet />
    </>
  )

  if (!useChatSession) return content

  return <ChatSessionProvider origin="newtab">{content}</ChatSessionProvider>
}
