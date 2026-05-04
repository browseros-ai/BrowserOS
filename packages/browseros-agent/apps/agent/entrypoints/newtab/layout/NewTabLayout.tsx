import type { FC } from 'react'
import { Outlet, useLocation } from 'react-router'
import { ChatSessionProvider } from '@/entrypoints/sidepanel/layout/ChatSessionContext'
import { NewTabFocusGrid } from './NewTabFocusGrid'
import { shouldHideFocusGrid } from './route-utils'

export const NewTabLayout: FC = () => {
  const location = useLocation()
  const hideGrid = shouldHideFocusGrid(location.pathname)
  const content = (
    <>
      {!hideGrid && <NewTabFocusGrid />}
      <Outlet />
    </>
  )

  return (
    <ChatSessionProvider origin="newtab">
      {content}
    </ChatSessionProvider>
  )
}
