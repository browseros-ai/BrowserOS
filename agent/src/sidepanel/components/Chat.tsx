import React, { useState, useRef } from 'react'
import { Header } from './Header'
import { MessageList } from './MessageList'
import { ChatInput } from './ChatInput'
import { SelectTabsButton } from './SelectTabsButton'
import { useChatStore } from '../stores/chatStore'
import { ErrorBoundary } from './ErrorBoundary'
import { useAnalytics } from '../hooks/useAnalytics'
import { ArrowDown } from 'lucide-react'
import { useSidePanelPortMessaging } from '@/sidepanel/hooks'

interface ChatProps {
  isConnected: boolean
}

/**
 * Main chat container component
 * Orchestrates the layout and manages the overall chat interface
 */
export function Chat({ isConnected }: ChatProps) {
  const { messages, isProcessing, reset, upsertMessage } = useChatStore()
  const [isUserScrolling, setIsUserScrolling] = useState(false)
  const [showSelectTabsButton, setShowSelectTabsButton] = useState(false)
  const messageListRef = useRef<HTMLDivElement>(null)
  const { trackFeature } = useAnalytics()
  const { addMessageListener, removeMessageListener } = useSidePanelPortMessaging()

  const handleScrollToBottom = () => {
    if (messageListRef.current) {
      messageListRef.current.scrollTo({
        top: messageListRef.current.scrollHeight,
        behavior: 'smooth'
      })
      setIsUserScrolling(false)
    }
  }

  const toggleSelectTabsButton = () => {
    setShowSelectTabsButton(prev => !prev)
  }

  // Note: MCP server status messages are handled by the Header component's toast notifications
  // We don't show them in the chat to keep the conversation clean

  return (
    <div className="flex flex-col h-full bg-transparent">

      {/* Header */}
      <Header 
        onReset={reset}
        showReset={messages.length > 0}
        isProcessing={isProcessing}
      />
      
      {/* Main content - takes remaining space and scrolls */}
      <div className="flex-1 min-h-0">
        <ErrorBoundary
          fallback={(error, reset) => (
            <div className="flex-1 flex items-center justify-center p-4">
              <div className="text-center">
                <p className="text-red-600 dark:text-red-400 mb-2">Failed to load messages</p>
                <button 
                  onClick={reset}
                  className="text-sm text-blue-600 hover:text-blue-800 dark:text-blue-400 dark:hover:text-blue-300"
                >
                  Try Again
                </button>
              </div>
            </div>
          )}
        >
          <main id="main-content" className="h-full min-h-0">
            
            <MessageList 
              messages={messages} 
              isProcessing={isProcessing}
              onScrollStateChange={setIsUserScrolling}
              scrollToBottom={handleScrollToBottom}
              containerRef={messageListRef}
            />
          </main>

        </ErrorBoundary>
      </div>
      
      {/* Scroll to bottom button - positioned above SelectTabsButton */}
      {isUserScrolling && (
        <div className="relative h-0">
          <button
            onClick={() => {
              trackFeature('scroll_to_bottom')
              handleScrollToBottom()
            }}
            className="absolute -top-12 right-4 p-3 bg-brand text-white rounded-full shadow-xl hover:bg-brand/90 transition-all duration-200 animate-in fade-in slide-in-from-bottom-2 focus-visible:outline-none z-10 border-2 border-white/20 hover:scale-110"
            aria-label="Scroll to bottom of messages"
            type="button"
          >
            <ArrowDown className="w-4 h-4" />
          </button>
        </div>
      )}
      
      {/* Select Tabs Button - conditionally rendered */}
      {showSelectTabsButton && (
        <ErrorBoundary
          fallback={(error, reset) => (
            <div className="px-4 py-2 bg-red-50 dark:bg-red-950/20">
              <p className="text-sm text-red-600 dark:text-red-400">
                Tab selector error. <button onClick={reset} className="underline">Reset</button>
              </p>
            </div>
          )}
        >
          <SelectTabsButton />
        </ErrorBoundary>
      )}
      
      {/* Chat Input - always at bottom */}
      <ErrorBoundary
        fallback={(error, reset) => (
          <div className="p-4 bg-red-50 dark:bg-red-950/20">
            <p className="text-sm text-red-600 dark:text-red-400">
              Input error. <button onClick={reset} className="underline">Reset</button>
            </p>
          </div>
        )}
      >
        <ChatInput 
          isConnected={isConnected}
          isProcessing={isProcessing}
          onToggleSelectTabs={toggleSelectTabsButton}
          showSelectTabsButton={showSelectTabsButton}
        />
      </ErrorBoundary>
      
    </div>
  )
}