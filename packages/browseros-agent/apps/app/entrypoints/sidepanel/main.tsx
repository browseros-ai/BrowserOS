import React from 'react'
import ReactDOM from 'react-dom/client'
import '@/styles/global.css'
import { ThemeProvider } from '@/components/theme-provider.tsx'
import { Toaster } from '@/components/ui/sonner'
import { AnalyticsProvider } from '@/lib/analytics/AnalyticsProvider'
import { QueryProvider } from '@/lib/query/QueryProvider'
import { sentryRootErrorHandler } from '@/lib/sentry/sentryRootErrorHandler'
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
