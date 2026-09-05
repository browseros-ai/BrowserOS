import { useEffect, useState } from 'react'
import { createBrowserOSAction } from '@/lib/chat-actions/types'
import { track } from '@/lib/metrics/track'
import { useChatSessionContext } from '@/modules/chat/chat-session-context'
import type { ChatMode } from '@/modules/chat/chat-types'

export interface ChatActionsConfig {
  /** Analytics event names scoped to the origin */
  events: {
    modeChanged: string
    stopClicked: string
    suggestionClicked: string
    tabToggled: string
    tabRemoved: string
    aiTriggered: string
  }
  /** Auto-attach current active tab on mount (sidepanel only) */
  autoAttachActiveTab?: boolean
}

export function useChatActions(config: ChatActionsConfig) {
  const session = useChatSessionContext()
  const { mode, setMode, sendMessage, stop, messages } = session

  const [input, setInput] = useState('')
  const [attachedTabs, setAttachedTabs] = useState<chrome.tabs.Tab[]>([])
  const [mounted, setMounted] = useState(false)

  useEffect(() => {
    setMounted(true)
  }, [])

  // Auto-attach current tab on mount (sidepanel)
  useEffect(() => {
    if (!config.autoAttachActiveTab) return
    ;(async () => {
      const currentTab = (
        await chrome.tabs.query({ active: true, currentWindow: true })
      ).filter((tab) => tab.url?.startsWith('http'))
      setAttachedTabs(currentTab)
    })()
  }, [config.autoAttachActiveTab])

  const handleModeChange = (newMode: ChatMode) => {
    track(config.events.modeChanged, { from: mode, to: newMode })
    setMode(newMode)
  }

  const handleStop = () => {
    track(config.events.stopClicked)
    stop()
  }

  const toggleTabSelection = (tab: chrome.tabs.Tab) => {
    setAttachedTabs((prev) => {
      const isSelected = prev.some((t) => t.id === tab.id)
      track(config.events.tabToggled, {
        action: isSelected ? 'removed' : 'added',
      })
      if (isSelected) {
        return prev.filter((t) => t.id !== tab.id)
      }
      return [...prev, tab]
    })
  }

  const removeTab = (tabId?: number) => {
    track(config.events.tabRemoved)
    setAttachedTabs((prev) => prev.filter((t) => t.id !== tabId))
  }

  const executeMessage = (customMessageText?: string) => {
    const messageText = customMessageText ? customMessageText : input.trim()
    if (!messageText) return

    if (attachedTabs.length) {
      const action = createBrowserOSAction({
        mode,
        message: messageText,
        tabs: attachedTabs,
      })
      sendMessage({ text: messageText, action })
    } else {
      sendMessage({ text: messageText })
    }
    setInput('')
    setAttachedTabs([])
  }

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault()
    if (messages.length === 0) {
      track(config.events.aiTriggered, {
        mode,
        tabs_count: attachedTabs.length,
      })
    }
    executeMessage()
  }

  const handleSuggestionClick = (suggestion: string) => {
    track(config.events.suggestionClicked, { mode })
    executeMessage(suggestion)
  }

  const { stop: _stop, ...restSession } = session

  return {
    ...restSession,
    input,
    setInput,
    attachedTabs,
    setAttachedTabs,
    mounted,
    handleModeChange,
    handleStop,
    toggleTabSelection,
    removeTab,
    executeMessage,
    handleSubmit,
    handleSuggestionClick,
  }
}
