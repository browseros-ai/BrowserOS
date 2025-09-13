import React, { useEffect } from 'react'
import { useMessageHandler } from './hooks/useMessageHandler'
import { useSidePanelPortMessaging } from '@/sidepanel/hooks'
import { Chat } from './components/Chat'
import { ErrorBoundary } from './components/ErrorBoundary'
import { useAnnouncer, setGlobalAnnouncer } from './hooks/useAnnouncer'
import { SkipLink } from './components/SkipLink'
import { useSettingsStore } from './stores/settingsStore'
import { HumanInputDialog } from './components/HumanInputDialog'
import './styles.css'

/**
 * Root component for sidepanel v2
 * Uses Tailwind CSS for styling
 */
export function App() {
  // Initialize message handling
  const { humanInputRequest, clearHumanInputRequest } = useMessageHandler()
  
  // Get connection status from port messaging
  const { connected } = useSidePanelPortMessaging()
  
  // Initialize settings
  const { fontSize, theme } = useSettingsStore()
  
  // Initialize global announcer for screen readers
  const announcer = useAnnouncer()
  useEffect(() => {
    setGlobalAnnouncer(announcer)
  }, [announcer])
  
  // Initialize settings on app load
  useEffect(() => {
    // Apply font size
    document.documentElement.style.setProperty('--app-font-size', `${fontSize}px`)
    
    // Apply theme classes
    const root = document.documentElement
    root.classList.remove('dark')
    root.classList.remove('gray')
    if (theme === 'dark') root.classList.add('dark')
    if (theme === 'gray') root.classList.add('gray')
  }, [fontSize, theme])
  
  // Announce connection status changes
  useEffect(() => {
    announcer.announce(connected ? 'Extension connected' : 'Extension disconnected')
  }, [connected, announcer])
  
  return (
    <ErrorBoundary
      onError={(error, errorInfo) => {
        // Log to analytics or error reporting service
        console.error('App level error:', error, errorInfo)
        announcer.announce('An error occurred. Please try again.', 'assertive')
      }}
    >
      <div className="h-screen bg-background overflow-hidden" role="main" aria-label="BrowserOS Chat Assistant">
        <SkipLink />
        <Chat isConnected={connected} />
        {humanInputRequest && (
          <HumanInputDialog
            requestId={humanInputRequest.requestId}
            prompt={humanInputRequest.prompt}
            onClose={clearHumanInputRequest}
          />
        )}
      </div>
    </ErrorBoundary>
  )
}