import React, { useEffect } from 'react'
import { useMessageHandler } from './hooks/useMessageHandler'
import { useSidePanelPortMessaging } from '@/sidepanel/hooks'
import { Chat } from './components/Chat'
import { ErrorBoundary } from './components/ErrorBoundary'
import { useAnnouncer, setGlobalAnnouncer } from './hooks/useAnnouncer'
import { SkipLink } from './components/SkipLink'
import { useSettingsStore } from './stores/settingsStore'
import { HumanInputDialog } from './components/HumanInputDialog'
import {
  GlowingStarsBackgroundCard,
  // GlowingStarsBackground,
} from './components/glowing-stars'
import './styles.css'

/**
 * Root component for sidepanel v2
 * Uses Tailwind CSS for styling
 */
export function App() {
  // Get connection status from port messaging
  const { connected } = useSidePanelPortMessaging()

  // Initialize message handling
  const { humanInputRequest, clearHumanInputRequest } = useMessageHandler()

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
    if (theme === 'dark') root.classList.add('dark')
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
      <GlowingStarsBackgroundCard className="h-screen relative overflow-hidden" isBackground={true}>
        <div className="h-screen bg-[#191919] relative overflow-hidden" role="main" aria-label="Nemo Chat Assistant"
        >
          {/* Main Content - Interactive layer */}
          <div className="relative z-20 h-full flex flex-col">
            <SkipLink />
            <div className="flex-1 flex items-center justify-center">
              <div className="w-full max-w-2xl h-full flex flex-col gap-6">
                <div className="flex-1 min-h-0">
                  <Chat isConnected={connected} />
                </div>
                {humanInputRequest && (
                  <HumanInputDialog
                    requestId={humanInputRequest.requestId}
                    prompt={humanInputRequest.prompt}
                    onClose={clearHumanInputRequest}
                  />
                )}
              </div>
            </div>
          </div>
        </div>
      </GlowingStarsBackgroundCard>
    </ErrorBoundary>
  )
}

const Icon = () => {
  return (
    <svg
      xmlns="http://www.w3.org/2000/svg"
      fill="none"
      viewBox="0 0 24 24"
      strokeWidth="1.5"
      stroke="currentColor"
      className="h-4 w-4 text-white stroke-2"
    >
      <path
        strokeLinecap="round"
        strokeLinejoin="round"
        d="M17.25 8.25L21 12m0 0l-3.75 3.75M21 12H3"
      />
    </svg>
  )
}