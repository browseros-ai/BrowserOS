import type ReactDOM from 'react-dom/client'
import { sentry } from './sentry'

/**
 * @public
 */
export const sentryRootErrorHandler: ReactDOM.RootOptions = {
  // Callback called when an error is thrown and not caught by an ErrorBoundary.
  onUncaughtError: sentry.reactErrorHandler((error, errorInfo) => {
    // Error caught by sentry, log to console for debugging purposes
    // biome-ignore lint/suspicious/noConsole: must log to console for debugging purposes
    console.error(error, errorInfo.componentStack)
  }),
  // Callback called when React catches an error in an ErrorBoundary.
  onCaughtError: sentry.reactErrorHandler(),
  // Callback called when React automatically recovers from errors.
  onRecoverableError: sentry.reactErrorHandler(),
}
