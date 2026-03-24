import type { FC } from 'react'
import { HashRouter, Route, Routes } from 'react-router'
import { ChatHistory } from './history/ChatHistory'
import { Chat } from './index/Chat'
import { ChatLayout } from './layout/ChatLayout'

export const App: FC = () => {
  return (
    <HashRouter>
      <Routes>
        <Route element={<ChatLayout />}>
          <Route index element={<Chat />} />
          <Route path="history" element={<ChatHistory />} />
          <Route path="*" element={<Chat />} />
        </Route>
      </Routes>
    </HashRouter>
  )
}
