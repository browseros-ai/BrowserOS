import { useEffect, useRef, useState } from 'react'
import { createBrowserOSAction } from '@/lib/chat-actions/types'
import { track } from '@/lib/metrics/track'
import { useChatSessionContext } from '@/modules/chat/chat-session-context'
import type { ChatMode } from '@/modules/chat/chat-types'
import { useVoiceInput } from '@/modules/voice/voice.hooks'
import {
  type ChatSessionLike,
  useVoiceLoop,
} from '@/modules/voice/voice-loop.hooks'

export interface ChatActionsConfig {
  /** Analytics event names scoped to the origin */
  events: {
    modeChanged: string
    stopClicked: string
    suggestionClicked: string
    tabToggled: string
    tabRemoved: string
    aiTriggered: string
    voiceRecordingStarted: string
    voiceRecordingStopped: string
    voiceTranscriptionCompleted: string
    voiceError: string
  }
  /** Auto-attach current active tab on mount (sidepanel only) */
  autoAttachActiveTab?: boolean
}

export function useChatActions(config: ChatActionsConfig) {
  const session = useChatSessionContext()
  const { mode, setMode, sendMessage, stop, messages, status } = session

  const voice = useVoiceInput()
  // Stable ref the voice loop reads on demand. Mutating .current is a
  // plain property write, not a React state change, so the chat
  // session updating on every streamed token does not propagate into
  // the voice loop's render cycle.
  const chatSessionRef = useRef<ChatSessionLike | null>(null)
  chatSessionRef.current = { sendMessage, stop, status, messages }
  const voiceLoop = useVoiceLoop({ chatSessionRef })

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

  // Voice transcript → input
  // biome-ignore lint/correctness/useExhaustiveDependencies: only trigger on transcript/transcribing change
  useEffect(() => {
    if (voice.transcript && !voice.isTranscribing) {
      setInput((prev) => {
        const separator = prev.trim() ? ' ' : ''
        return prev + separator + voice.transcript
      })
      track(config.events.voiceTranscriptionCompleted)
      voice.clearTranscript()
    }
  }, [voice.transcript, voice.isTranscribing])

  // Track voice errors
  useEffect(() => {
    if (voice.error) {
      track(config.events.voiceError, { error: voice.error })
    }
  }, [voice.error, config.events.voiceError])

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

  const handleStartRecording = async () => {
    const started = await voice.startRecording()
    if (started) {
      track(config.events.voiceRecordingStarted)
    }
  }

  const handleStopRecording = async () => {
    await voice.stopRecording()
    track(config.events.voiceRecordingStopped)
  }

  const voiceState = {
    isRecording: voice.isRecording,
    isTranscribing: voice.isTranscribing,
    audioLevels: voice.audioLevels,
    error: voice.error,
    onStartRecording: handleStartRecording,
    onStopRecording: handleStopRecording,
  }

  const { stop: _stop, ...restSession } = session

  return {
    ...restSession,
    input,
    setInput,
    attachedTabs,
    setAttachedTabs,
    mounted,
    voiceState,
    voiceLoop,
    handleModeChange,
    handleStop,
    toggleTabSelection,
    removeTab,
    executeMessage,
    handleSubmit,
    handleSuggestionClick,
  }
}
