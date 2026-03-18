import type { FC } from 'react'
import { Outlet, useLocation } from 'react-router'
import { ChatSessionProvider } from '@/entrypoints/sidepanel/layout/ChatSessionContext'
import { NewTabFocusGrid } from './NewTabFocusGrid'

const HIDE_FOCUS_GRID_PATHS = new Set([
  '/home/soul',
  '/home/memory',
  '/home/skills',
  '/home/chat',
])

export const NewTabLayout: FC = () => {
  const location = useLocation()

  return (
    <ChatSessionProvider origin="newtab">
      {!HIDE_FOCUS_GRID_PATHS.has(location.pathname) && <NewTabFocusGrid />}
      <Outlet />
    </ChatSessionProvider>
  )
}
