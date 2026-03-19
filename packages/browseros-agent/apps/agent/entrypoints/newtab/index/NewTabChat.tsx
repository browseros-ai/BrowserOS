import { Loader2 } from 'lucide-react'
import { type FC, useEffect, useRef } from 'react'
import { useSearchParams } from 'react-router'
import { ChatEmptyState } from '@/entrypoints/sidepanel/index/ChatEmptyState'
import { ChatError } from '@/entrypoints/sidepanel/index/ChatError'
import { ChatFooter } from '@/entrypoints/sidepanel/index/ChatFooter'
import { ChatHeader } from '@/entrypoints/sidepanel/index/ChatHeader'
import { ChatMessages } from '@/entrypoints/sidepanel/index/ChatMessages'
import {
  createAITabAction,
  createBrowserOSAction,
} from '@/lib/chat-actions/types'
import { useChatActions } from '@/lib/chat-actions/useChatActions'
import {
  NEWTAB_AI_TRIGGERED_EVENT,
  NEWTAB_CHAT_MODE_CHANGED_EVENT,
  NEWTAB_CHAT_RESET_EVENT,
  NEWTAB_CHAT_STOPPED_EVENT,
  NEWTAB_CHAT_SUGGESTION_CLICKED_EVENT,
  NEWTAB_TAB_REMOVED_EVENT,
  NEWTAB_TAB_TOGGLED_EVENT,
  NEWTAB_VOICE_ERROR_EVENT,
  NEWTAB_VOICE_RECORDING_STARTED_EVENT,
  NEWTAB_VOICE_RECORDING_STOPPED_EVENT,
  NEWTAB_VOICE_TRANSCRIPTION_COMPLETED_EVENT,
} from '@/lib/constants/analyticsEvents'
import { track } from '@/lib/metrics/track'

export const NewTabChat: FC = () => {
  const [searchParams, setSearchParams] = useSearchParams()
  const hasSentInitialRef = useRef(false)

  const {
    mode,
    setMode,
    messages,
    sendMessage,
    status,
    agentUrlError,
    chatError,
    getActionForMessage,
    liked,
    onClickLike,
    disliked,
    onClickDislike,
    isRestoringConversation,
    providers,
    selectedProvider,
    handleSelectProvider,
    resetConversation,
    input,
    setInput,
    attachedTabs,
    mounted,
    voiceState,
    handleModeChange,
    handleStop,
    toggleTabSelection,
    removeTab,
    handleSubmit,
    handleSuggestionClick,
  } = useChatActions({
    events: {
      modeChanged: NEWTAB_CHAT_MODE_CHANGED_EVENT,
      stopClicked: NEWTAB_CHAT_STOPPED_EVENT,
      suggestionClicked: NEWTAB_CHAT_SUGGESTION_CLICKED_EVENT,
      tabToggled: NEWTAB_TAB_TOGGLED_EVENT,
      tabRemoved: NEWTAB_TAB_REMOVED_EVENT,
      aiTriggered: NEWTAB_AI_TRIGGERED_EVENT,
      voiceRecordingStarted: NEWTAB_VOICE_RECORDING_STARTED_EVENT,
      voiceRecordingStopped: NEWTAB_VOICE_RECORDING_STOPPED_EVENT,
      voiceTranscriptionCompleted: NEWTAB_VOICE_TRANSCRIPTION_COMPLETED_EVENT,
      voiceError: NEWTAB_VOICE_ERROR_EVENT,
    },
  })

  // Send the initial message from URL query params (from /home search bar).
  // Guarded by ref to prevent double-fire in React Strict Mode.
  // biome-ignore lint/correctness/useExhaustiveDependencies: must only run once on mount
  useEffect(() => {
    if (hasSentInitialRef.current) return
    const query = searchParams.get('q')
    const chatMode = searchParams.get('mode')
    const tabIdsParam = searchParams.get('tabs')
    if (!query) return

    hasSentInitialRef.current = true
    if (chatMode === 'chat' || chatMode === 'agent') {
      setMode(chatMode)
    }
    setSearchParams({}, { replace: true })

    const actionType = searchParams.get('actionType')
    const tabName = searchParams.get('tabName')
    const tabDescription = searchParams.get('tabDescription')

    if (tabIdsParam) {
      const tabIds = tabIdsParam.split(',').map(Number).filter(Boolean)
      chrome.tabs.query({}).then((allTabs) => {
        const matchedTabs = allTabs.filter(
          (t) => t.id !== undefined && tabIds.includes(t.id),
        )
        if (matchedTabs.length > 0) {
          const action =
            actionType === 'ai-tab' && tabName
              ? createAITabAction({
                  name: tabName,
                  description: tabDescription ?? '',
                  tabs: matchedTabs,
                })
              : createBrowserOSAction({
                  mode: (chatMode as 'chat' | 'agent') ?? 'agent',
                  message: query,
                  tabs: matchedTabs,
                })
          sendMessage({ text: query, action })
        } else {
          sendMessage({ text: query })
        }
      })
    } else {
      sendMessage({ text: query })
    }
  }, [])

  const handleNewConversation = () => {
    track(NEWTAB_CHAT_RESET_EVENT, { message_count: messages.length })
    resetConversation()
  }

  if (!selectedProvider) return null

  return (
    <div className="absolute inset-0 flex flex-col overflow-hidden">
      <div className="mx-auto w-full max-w-3xl">
        <ChatHeader
          selectedProvider={selectedProvider}
          providers={providers}
          onSelectProvider={handleSelectProvider}
          onNewConversation={handleNewConversation}
          hasMessages={messages.length > 0}
          hideHistory
        />
      </div>

      <main className="styled-scrollbar [&_[data-streamdown='code-block']]:!max-w-full [&_[data-streamdown='code-block']]:!w-auto [&_[data-streamdown='table-wrapper']]:!max-w-full [&_[data-streamdown='table-wrapper']]:!w-auto mx-auto flex min-h-0 w-full max-w-3xl flex-1 flex-col space-y-4 overflow-y-auto overflow-x-hidden px-4 pt-4 [&_[data-streamdown='code-block']]:overflow-x-auto [&_[data-streamdown='table-wrapper']]:overflow-x-auto">
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
            showJtbdPopup={false}
            showDontShowAgain={false}
            onTakeSurvey={() => {}}
            onDismissJtbdPopup={() => {}}
          />
        )}
        {agentUrlError && <ChatError error={agentUrlError} />}
        {chatError && <ChatError error={chatError} />}
      </main>

      <div className="mx-auto w-full max-w-3xl flex-shrink-0 px-4 pb-2">
        <ChatFooter
          mode={mode}
          onModeChange={handleModeChange}
          input={input}
          onInputChange={setInput}
          onSubmit={handleSubmit}
          status={status}
          onStop={handleStop}
          attachedTabs={attachedTabs}
          onToggleTab={toggleTabSelection}
          onRemoveTab={removeTab}
          voice={voiceState}
        />
      </div>
    </div>
  )
}
