import type { FC } from 'react'
import { HashRouter, Route, Routes } from 'react-router'
import { ChatLayout } from '@/components/layout/ChatLayout'
import { ChatHistory } from '@/screens/sidepanel/history/ChatHistory'
import { Chat } from '@/screens/sidepanel/index/Chat'

export const App: FC = () => {
  return (
    <HashRouter>
      <Routes>
        <Route element={<ChatLayout />}>
          <Route index element={<Chat />} />
          <Route path="history" element={<ChatHistory />} />
        </Route>
      </Routes>
    </HashRouter>
  )
}
