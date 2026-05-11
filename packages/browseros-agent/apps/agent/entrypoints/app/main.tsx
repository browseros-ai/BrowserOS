import React from 'react'
import ReactDOM from 'react-dom/client'
import '@/styles/global.css'
import { ThemeProvider } from '@/components/theme-provider.tsx'
import { Toaster } from '@/components/ui/sonner'
import { AnalyticsProvider } from '@/lib/analytics/AnalyticsProvider.tsx'
import { AuthProvider } from '@/lib/auth/AuthProvider'
import { QueryProvider } from '@/lib/graphql/QueryProvider'
import { sentryRootErrorHandler } from '@/lib/sentry/sentryRootErrorHandler.ts'
import { App } from './App'

type AppErrorBoundaryState = {
  hasError: boolean
  message?: string
}

class AppErrorBoundary extends React.Component<
  React.PropsWithChildren,
  AppErrorBoundaryState
> {
  state: AppErrorBoundaryState = { hasError: false }

  static getDerivedStateFromError(error: unknown): AppErrorBoundaryState {
    return {
      hasError: true,
      message: error instanceof Error ? error.message : String(error),
    }
  }

  componentDidCatch(error: unknown): void {
    // Keep the app visible with a fallback instead of rendering a blank screen.
    // biome-ignore lint/suspicious/noConsole: Log render failures for debugging (also visible in UI).
    console.error('App render error:', error)
  }

  render(): React.ReactNode {
    if (this.state.hasError) {
      return (
        <div className="m-4 rounded-lg border border-destructive/30 bg-destructive/5 p-4 text-sm">
          <div className="mb-2 font-medium">Sup failed to render</div>
          <div className="text-muted-foreground">
            {this.state.message ?? 'Unknown render error'}
          </div>
          <div className="mt-3 text-muted-foreground text-xs">
            Reload the extension page after dev server restart.
          </div>
        </div>
      )
    }
    return this.props.children
  }
}

const $root = document.getElementById('root')

if ($root) {
  ReactDOM.createRoot($root, sentryRootErrorHandler).render(
    <React.StrictMode>
      <AppErrorBoundary>
        <AuthProvider>
          <QueryProvider>
            <AnalyticsProvider>
              <ThemeProvider>
                <App />
                <Toaster />
              </ThemeProvider>
            </AnalyticsProvider>
          </QueryProvider>
        </AuthProvider>
      </AppErrorBoundary>
    </React.StrictMode>,
  )
}
