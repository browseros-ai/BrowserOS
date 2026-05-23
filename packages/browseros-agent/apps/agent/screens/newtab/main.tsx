import React from 'react'
import ReactDOM from 'react-dom/client'
import '@/styles/global.css'
import { HashRouter } from 'react-router'
import { ThemeProvider } from '@/components/theme-provider.tsx'
import { Toaster } from '@/components/ui/sonner'
import { ChatSessionProvider } from '@/entrypoints/sidepanel/layout/ChatSessionContext'
import { AnalyticsProvider } from '@/lib/analytics/AnalyticsProvider'
import { AuthProvider } from '@/lib/auth/AuthProvider'
import { QueryProvider } from '@/lib/graphql/QueryProvider'
import { NewTabApp } from './NewTabApp'

const $root = document.getElementById('root')

if ($root) {
  ReactDOM.createRoot($root).render(
    <React.StrictMode>
      <HashRouter>
        <AuthProvider>
          <QueryProvider>
            <AnalyticsProvider>
              <ThemeProvider>
                <ChatSessionProvider origin="newtab">
                  <NewTabApp />
                  <Toaster />
                </ChatSessionProvider>
              </ThemeProvider>
            </AnalyticsProvider>
          </QueryProvider>
        </AuthProvider>
      </HashRouter>
    </React.StrictMode>,
  )
}
