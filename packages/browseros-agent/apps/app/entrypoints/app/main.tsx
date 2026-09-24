import React from 'react'
import ReactDOM from 'react-dom/client'
import '@/styles/global.css'
import { ThemeProvider } from '@/components/theme-provider.tsx'
import { Toaster } from '@/components/ui/sonner'
import { AnalyticsProvider } from '@/lib/analytics/AnalyticsProvider.tsx'
import { QueryProvider } from '@/lib/query/QueryProvider'
import { sentryRootErrorHandler } from '@/lib/sentry/sentryRootErrorHandler.ts'
import { App } from './App'

const $root = document.getElementById('root')

if ($root) {
  ReactDOM.createRoot($root, sentryRootErrorHandler).render(
    <React.StrictMode>
      <QueryProvider>
        <AnalyticsProvider>
          <ThemeProvider>
            <App />
            <Toaster />
          </ThemeProvider>
        </AnalyticsProvider>
      </QueryProvider>
    </React.StrictMode>,
  )
}
