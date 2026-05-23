import type { FC } from 'react'
import { Outlet, useLocation } from 'react-router'
import { ChatSessionProvider } from '@/entrypoints/sidepanel/layout/ChatSessionContext'
import { NewTabFocusGrid } from './NewTabFocusGrid'
import { shouldHideFocusGrid, shouldUseChatSession } from './route-utils'

interface NewTabLayoutProps {
  useChatSessionOnHome?: boolean
}

export const NewTabLayout: FC<NewTabLayoutProps> = ({
  useChatSessionOnHome = false,
}) => {
  const location = useLocation()
  const hideGrid = shouldHideFocusGrid(location.pathname)
  const useChatSession = shouldUseChatSession(
    location.pathname,
    useChatSessionOnHome,
  )
  const content = (
    <>
      {!hideGrid && <NewTabFocusGrid />}
      <Outlet />
    </>
  )

  // Security Hardening & Fix: Always wrap /home in ChatSessionProvider because NewTab component requires it
  if (!useChatSession && location.pathname !== '/home') return content

  return <ChatSessionProvider origin="newtab">{content}</ChatSessionProvider>
}
