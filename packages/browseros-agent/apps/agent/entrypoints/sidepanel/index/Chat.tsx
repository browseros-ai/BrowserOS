import { Loader2 } from 'lucide-react'
import { useEffect, useRef, useState } from 'react'
import { createBrowserOSAction } from '@/lib/chat-actions/types'
import {
  SIDEPANEL_AI_TRIGGERED_EVENT,
  SIDEPANEL_MODE_CHANGED_EVENT,
  SIDEPANEL_STOP_CLICKED_EVENT,
  SIDEPANEL_SUGGESTION_CLICKED_EVENT,
  SIDEPANEL_TAB_REMOVED_EVENT,
  SIDEPANEL_TAB_TOGGLED_EVENT,
  SIDEPANEL_VOICE_ERROR_EVENT,
  SIDEPANEL_VOICE_RECORDING_STARTED_EVENT,
  SIDEPANEL_VOICE_RECORDING_STOPPED_EVENT,
  SIDEPANEL_VOICE_TRANSCRIPTION_COMPLETED_EVENT,
  SLASH_COMMAND_EXECUTED_EVENT,
} from '@/lib/constants/analyticsEvents'
import { useJtbdPopup } from '@/lib/jtbd-popup/useJtbdPopup'
import { track } from '@/lib/metrics/track'
import { useVoiceInput } from '@/lib/voice/useVoiceInput'
import { processSlashCommand, getAllCommands, type SlashCommand } from '@/lib/slash-commands'
import { registerBuiltinCommands } from '@/lib/slash-commands/builtins'
import { useChatSessionContext } from '../layout/ChatSessionContext'
import { ChatEmptyState } from './ChatEmptyState'
import { ChatError } from './ChatError'
import { ChatFooter } from './ChatFooter'
import { ChatMessages } from './ChatMessages'
import type { ChatMode } from './chatTypes'

/**
 * @public
 */
export const Chat = () => {
  const {
    mode,
    setMode,
    messages,
    sendMessage,
    status,
    stop,
    agentUrlError,
    chatError,
    selectedProvider,
    getActionForMessage,
    liked,
    onClickLike,
    disliked,
    onClickDislike,
    isRestoringConversation,
    addToolApprovalResponse,
    undoTurn,
    forkTurn,
    editTurn,
    conversationId,
    setMessages: setMessagesFromContext,
    resetConversation,
  } = useChatSessionContext()

  // Register slash commands once on mount
  const slashCommandsRegistered = useRef(false)
  if (!slashCommandsRegistered.current) {
    registerBuiltinCommands()
    slashCommandsRegistered.current = true
  }

  const [slashCommands] = useState<SlashCommand[]>(() => getAllCommands())

  const {
    popupVisible,
    showDontShowAgain,
    recordMessageSent,
    triggerIfEligible,
    onTakeSurvey,
    onDismiss: onDismissJtbdPopup,
  } = useJtbdPopup()

  const voice = useVoiceInput()

  const [input, setInput] = useState('')
  const [attachedTabs, setAttachedTabs] = useState<chrome.tabs.Tab[]>([])
  const [mounted, setMounted] = useState(false)
  const [slashCommandOpen, setSlashCommandOpen] = useState(false)
  const [slashFilterText, setSlashFilterText] = useState('')

  useEffect(() => {
    setMounted(true)
  }, [])

  useEffect(() => {
    ;(async () => {
      const currentTab = (
        await chrome.tabs.query({
          active: true,
          currentWindow: true,
        })
      ).filter((tab) => tab.url?.startsWith('http'))
      setAttachedTabs(currentTab)
    })()
  }, [])

  // Trigger JTBD popup when AI finishes responding
  const previousChatStatus = useRef(status)
  // biome-ignore lint/correctness/useExhaustiveDependencies: intentionally only trigger on status change
  useEffect(() => {
    const aiWasProcessing =
      previousChatStatus.current === 'streaming' ||
      previousChatStatus.current === 'submitted'
    const aiJustFinished = aiWasProcessing && status === 'ready'

    if (aiJustFinished && messages.length > 0) {
      triggerIfEligible()
    }
    previousChatStatus.current = status
  }, [status])

  // Insert transcript into input when transcription completes
  // biome-ignore lint/correctness/useExhaustiveDependencies: only trigger on transcript/transcribing change
  useEffect(() => {
    if (voice.transcript && !voice.isTranscribing) {
      setInput((prev) => {
        const separator = prev.trim() ? ' ' : ''
        return prev + separator + voice.transcript
      })
      track(SIDEPANEL_VOICE_TRANSCRIPTION_COMPLETED_EVENT)
      voice.clearTranscript()
    }
  }, [voice.transcript, voice.isTranscribing])

  // Track voice errors
  useEffect(() => {
    if (voice.error) {
      track(SIDEPANEL_VOICE_ERROR_EVENT, { error: voice.error })
    }
  }, [voice.error])

  const handleInputChange = (value: string) => {
    setInput(value)

    // Track slash command autocomplete
    if (value.startsWith('/')) {
      const afterSlash = value.slice(1)
      const spaceIndex = afterSlash.indexOf(' ')
      if (spaceIndex === -1 && afterSlash.length > 0) {
        setSlashFilterText(afterSlash)
        setSlashCommandOpen(true)
      } else {
        setSlashCommandOpen(false)
      }
    } else {
      setSlashCommandOpen(false)
    }
  }

  const handleSlashSelect = (cmd: SlashCommand) => {
    const nextInput = `/${cmd.name} `
    setInput(nextInput)
    setSlashCommandOpen(false)
    setSlashFilterText('')
  }

  const handleModeChange = (newMode: ChatMode) => {
    track(SIDEPANEL_MODE_CHANGED_EVENT, { from: mode, to: newMode })
    setMode(newMode)
  }

  const handleStop = () => {
    track(SIDEPANEL_STOP_CLICKED_EVENT)
    stop()
  }

  const toggleTabSelection = (tab: chrome.tabs.Tab) => {
    setAttachedTabs((prev) => {
      const isSelected = prev.some((t) => t.id === tab.id)
      track(SIDEPANEL_TAB_TOGGLED_EVENT, {
        action: isSelected ? 'removed' : 'added',
      })
      if (isSelected) {
        return prev.filter((t) => t.id !== tab.id)
      }
      return [...prev, tab]
    })
  }

  const removeTab = (tabId?: number) => {
    track(SIDEPANEL_TAB_REMOVED_EVENT)
    setAttachedTabs((prev) => prev.filter((t) => t.id !== tabId))
  }

  const executeMessage = (customMessageText?: string) => {
    const messageText = customMessageText ? customMessageText : input.trim()
    if (!messageText) return

    recordMessageSent()

    // Process slash commands before sending
    if (messageText.startsWith('/')) {
      const result = processSlashCommand(messageText, {
        messages,
        conversationId,
        setMessages: setMessagesFromContext,
        resetConversation,
        mode,
        setMode,
      })

      // Handle async results (none currently, but future-proof)
      const resolved = result instanceof Promise ? null : result
      if (resolved) {
        if (resolved.type === 'action') {
          track(SLASH_COMMAND_EXECUTED_EVENT, { command: messageText.split(' ')[0], type: 'action' })
          setInput('')
          setAttachedTabs([])
          return
        }
        if (resolved.type === 'prompt') {
          track(SLASH_COMMAND_EXECUTED_EVENT, { command: messageText.split(' ')[0], type: 'prompt' })
          if (attachedTabs.length) {
            const action = createBrowserOSAction({
              mode,
              message: resolved.expandedText,
              tabs: attachedTabs,
            })
            sendMessage({ text: resolved.expandedText, action })
          } else {
            sendMessage({ text: resolved.expandedText })
          }
          setInput('')
          setAttachedTabs([])
          return
        }
        // passthrough — fall through to normal send
      }
    }

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
      track(SIDEPANEL_AI_TRIGGERED_EVENT, {
        mode,
        tabs_count: attachedTabs.length,
      })
    }
    executeMessage()
  }

  const handleSuggestionClick = (suggestion: string) => {
    track(SIDEPANEL_SUGGESTION_CLICKED_EVENT, { mode })
    executeMessage(suggestion)
  }

  const handleStartRecording = async () => {
    const started = await voice.startRecording()
    if (started) {
      track(SIDEPANEL_VOICE_RECORDING_STARTED_EVENT)
    }
  }

  const handleStopRecording = async () => {
    await voice.stopRecording()
    track(SIDEPANEL_VOICE_RECORDING_STOPPED_EVENT)
  }

  const voiceState = {
    isRecording: voice.isRecording,
    isTranscribing: voice.isTranscribing,
    audioLevels: voice.audioLevels,
    error: voice.error,
    onStartRecording: handleStartRecording,
    onStopRecording: handleStopRecording,
  }

  return (
    <>
      <main className="mt-4 flex h-full flex-1 flex-col space-y-4 overflow-y-auto">
        {isRestoringConversation ? (
          <div className="flex flex-1 items-center justify-center">
            <Loader2 className="h-6 w-6 animate-spin text-muted-foreground" />
          </div>
        ) : messages.length === 0 ? (
          <ChatEmptyState
            mode={mode}
            mounted={mounted}
            onSuggestionClick={handleSuggestionClick}
          />
        ) : (
          <ChatMessages
            messages={messages}
            status={status}
            getActionForMessage={getActionForMessage}
            liked={liked}
            onClickLike={onClickLike}
            disliked={disliked}
            onClickDislike={onClickDislike}
            showJtbdPopup={popupVisible}
            showDontShowAgain={showDontShowAgain}
            onTakeSurvey={onTakeSurvey}
            onDismissJtbdPopup={onDismissJtbdPopup}
            onToolApprove={(id) =>
              addToolApprovalResponse({ id, approved: true })
            }
            onToolDeny={(id) =>
              addToolApprovalResponse({ id, approved: false })
            }
            onUndoTurn={undoTurn}
            onForkTurn={forkTurn}
            onEditTurn={editTurn}
          />
        )}
        {agentUrlError && (
          <ChatError
            error={agentUrlError}
            providerType={selectedProvider?.type}
          />
        )}
        {chatError && (
          <ChatError error={chatError} providerType={selectedProvider?.type} />
        )}
      </main>

      <ChatFooter
        mode={mode}
        onModeChange={handleModeChange}
        input={input}
        onInputChange={handleInputChange}
        onSubmit={handleSubmit}
        status={status}
        onStop={handleStop}
        attachedTabs={attachedTabs}
        onToggleTab={toggleTabSelection}
        onRemoveTab={removeTab}
        voice={voiceState}
        slashCommands={slashCommands}
        onSlashSelect={handleSlashSelect}
        slashCommandOpen={slashCommandOpen}
        slashFilterText={slashFilterText}
      />
    </>
  )
}
