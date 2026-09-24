import { useChat } from '@ai-sdk/react'
import { useQueryClient } from '@tanstack/react-query'
import { DefaultChatTransport, type FileUIPart, type UIMessage } from 'ai'
import { compact } from 'es-toolkit/array'
import { useCallback, useEffect, useRef, useState } from 'react'
import { useSearchParams } from 'react-router'
import useDeepCompareEffect from 'use-deep-compare-effect'
import type { Provider } from '@/components/chat/chatComponentTypes'
import {
  conversationForTab,
  conversationPanelViewsStorage,
} from '@/lib/browseros/conversationPanelStorage'
import { isIncognitoWindow } from '@/lib/browseros/incognito'
import { resolvePanelTabId } from '@/lib/browseros/panelTab'
import type { ChatAction } from '@/lib/chat-actions/types'
import {
  CONVERSATION_RESET_EVENT,
  GLOW_STOP_CLICKED_EVENT,
  MESSAGE_DISLIKE_EVENT,
  MESSAGE_LIKE_EVENT,
  MESSAGE_SENT_EVENT,
  PROVIDER_SELECTED_EVENT,
} from '@/lib/constants/analyticsEvents'
import { formatConversationHistory } from '@/lib/conversations/formatConversationHistory'
import { declinedAppsStorage } from '@/lib/declined-apps/storage'
import { resolveChatProvider } from '@/lib/llm-providers/provider-runtime'
import type { ChatRequestBrowserContext } from '@/lib/messaging/server/buildChatRequestBody'
import { track } from '@/lib/metrics/track'
import { searchActionsStorage } from '@/lib/search-actions/searchActionsStorage'
import { selectedTextStorage } from '@/lib/selected-text/selectedTextStorage'
import { sentry } from '@/lib/sentry/sentry'
import { stopAgentStorage } from '@/lib/stop-agent/stop-agent-storage'
import { selectedWorkspaceStorage } from '@/lib/workspace/workspace-storage'
import { resolveAgentServerUrl } from '@/modules/browseros/agent-server-url.helpers'
import { useAgentServerUrl } from '@/modules/browseros/agent-server-url.hooks'
import {
  fetchServerConversation,
  SERVER_CONVERSATIONS_QUERY_KEY,
} from '@/modules/conversations/conversations.hooks'
import { useChatRefs } from './chat-refs.hooks'
import { decideChatSend, drainPendingSends } from './chat-send-decision'
import {
  didStreamingTurnFinish,
  getPersistableMessages,
  shouldPersistHistory,
} from './chat-session-persistence'
import {
  prepareSidepanelSendMessagesRequest,
  toProviderOption,
} from './chat-session-request'
import {
  resolveRestoredChatTarget,
  restoreServerConversation,
} from './chat-session-restore'
import type { ChatMode } from './chat-types'
import { addContentFilterNotice } from './content-filter-notice'
import {
  conversationReconnectUrl,
  fetchConversationRunState,
} from './conversation-run-client'
import { useExecutionHistoryTracker } from './execution-history-tracker.hooks'
import { PanelConversationAttachment } from './panel-conversation-attachment'
import { toLlmProviderConfig } from './sidepanel-chat-targets'
import { stripImageToolOutputs } from './tool-output-strip'

const getLastMessageText = (messages: UIMessage[]) => {
  const lastMessage = messages[messages.length - 1]
  if (!lastMessage) return ''
  return lastMessage.parts
    .filter((part) => part.type === 'text')
    .map((part) => part.text)
    .join('')
}

const getLastUserMessageText = (messages: UIMessage[]) => {
  for (let i = messages.length - 1; i >= 0; i -= 1) {
    if (messages[i]?.role === 'user') {
      return getLastMessageText([messages[i]])
    }
  }
  return ''
}

const getLastUserMessageFiles = (messages: UIMessage[]) => {
  for (let index = messages.length - 1; index >= 0; index -= 1) {
    const message = messages[index]
    if (message?.role === 'user') {
      return message.parts.filter((part) => part.type === 'file')
    }
  }
  return []
}

const getResponseAndQueryFromMessageId = (
  messages: UIMessage[],
  messageId: string,
) => {
  const messageIndex = messages.findIndex((each) => each.id === messageId)
  const response = messages?.[messageIndex] ?? []
  const query = messages?.[messageIndex - 1] ?? []
  const responseText = response.parts
    .filter((each) => each.type === 'text')
    .map((each) => each.text)
    .join('\n\n')
  const queryText = query.parts
    .filter((each) => each.type === 'text')
    .map((each) => each.text)
    .join('\n')

  return {
    responseText,
    queryText,
  }
}

export type ChatOrigin = 'sidepanel' | 'newtab'

export interface ChatSessionOptions {
  origin?: ChatOrigin
  /** When false, messages are queued until integrations finish syncing. */
  isIntegrationsSynced?: boolean
}

const NEWTAB_SYSTEM_PROMPT = `IMPORTANT: The user is chatting from the New Tab page. When performing browser actions, ALWAYS open content in a NEW TAB rather than navigating the current tab. The user's new tab page should remain accessible.`

const getUserSystemPrompt = (
  origin: ChatOrigin | undefined,
  personalization: string,
) =>
  origin === 'newtab'
    ? [personalization, NEWTAB_SYSTEM_PROMPT].filter(Boolean).join('\n\n')
    : personalization

const buildRequestBrowserContext = ({
  activeTab,
  action,
  enabledMcpServers,
  customMcpServers,
}: {
  activeTab?: chrome.tabs.Tab
  action?: ChatAction
  enabledMcpServers: Array<string | undefined>
  customMcpServers: {
    name: string
    url?: string
  }[]
}): ChatRequestBrowserContext | undefined => {
  const browserContext: ChatRequestBrowserContext = {}

  if (activeTab) {
    browserContext.windowId = activeTab.windowId
    browserContext.activeTab = {
      id: activeTab.id,
      url: activeTab.url,
      title: activeTab.title,
    }
  }

  if (action?.tabs?.length) {
    browserContext.selectedTabs = action.tabs.map((tab) => ({
      id: tab.id,
      url: tab.url,
      title: tab.title,
    }))
  }

  const managedMcpServers = compact(enabledMcpServers)
  if (managedMcpServers.length) {
    browserContext.enabledMcpServers = managedMcpServers
  }

  if (customMcpServers.length) {
    browserContext.customMcpServers = customMcpServers
  }

  return Object.keys(browserContext).length ? browserContext : undefined
}

export const useChatSession = (options?: ChatSessionOptions) => {
  const {
    selectedLlmProviderRef,
    selectedChatTargetRef,
    enabledMcpServersRef,
    enabledCustomServersRef,
    personalizationRef,
    chatTargets,
    selectedChatTarget,
    selectChatTarget,
    selectedLlmProvider,
    isLoadingProviders,
    hasAnyTarget,
    isSettled,
  } = useChatRefs()
  const queryClient = useQueryClient()

  // Incognito chats are never written to history or the cloud (#1189). Resolved
  // from the hosting window on mount (chrome.extension.inIncognitoContext is
  // false for a side panel in spanning mode). This settles long before any turn
  // ends, so the turn-end save always sees the correct value.
  const [isIncognito, setIsIncognito] = useState(false)
  useEffect(() => {
    let cancelled = false
    isIncognitoWindow().then((incognito) => {
      if (!cancelled) setIsIncognito(incognito)
    })
    return () => {
      cancelled = true
    }
  }, [])
  const persistHistory = shouldPersistHistory(isIncognito)

  const {
    baseUrl: agentServerUrl,
    isLoading: isLoadingAgentUrl,
    error: agentUrlError,
  } = useAgentServerUrl()

  const [searchParams, setSearchParams] = useSearchParams()
  const setSearchParamsRef = useRef(setSearchParams)
  setSearchParamsRef.current = setSearchParams
  const conversationIdParam = searchParams.get('conversationId')
  const [restoreError, setRestoreError] = useState<string | null>(null)
  const [restoreAttempt, setRestoreAttempt] = useState(0)
  const [restoredConversationId, setRestoredConversationId] = useState<
    string | null
  >(null)
  const isRestoringConversation =
    !!conversationIdParam && restoredConversationId !== conversationIdParam

  // Read via a ref because the transport closure below is created only once.
  const persistRef = useRef(persistHistory)
  useEffect(() => {
    persistRef.current = persistHistory
  }, [persistHistory])

  const agentUrlRef = useRef(agentServerUrl)

  useEffect(() => {
    agentUrlRef.current = agentServerUrl
  }, [agentServerUrl])

  // Read through a ref because the search-action watcher below installs once
  // and would otherwise keep the value from first render, when nothing has
  // loaded yet and every send would look unsendable.
  const hasAnyTargetRef = useRef(hasAnyTarget)
  const isSettledRef = useRef(isSettled)

  useEffect(() => {
    hasAnyTargetRef.current = hasAnyTarget
  }, [hasAnyTarget])

  useEffect(() => {
    isSettledRef.current = isSettled
  }, [isSettled])

  const [sendAttemptBlocked, setSendAttemptBlocked] = useState(false)
  // Derived rather than cleared in an effect: connecting a provider makes this
  // false on the next render on its own.
  const sendBlocked = sendAttemptBlocked && !hasAnyTarget

  const canSend =
    !isLoadingAgentUrl &&
    !agentUrlError &&
    !!agentServerUrl &&
    !isRestoringConversation &&
    !restoreError

  const providers: Provider[] = chatTargets.map(toProviderOption)

  const [mode, setMode] = useState<ChatMode>('agent')
  const [textToAction, setTextToAction] = useState<Map<string, ChatAction>>(
    new Map(),
  )
  const [liked, setLiked] = useState<Record<string, boolean>>({})
  const [disliked, setDisliked] = useState<Record<string, boolean>>({})
  const [conversationId, setConversationId] = useState(crypto.randomUUID())
  const conversationIdRef = useRef(conversationId)
  const optionsRef = useRef(options)
  const panelTabRef = useRef<Promise<number | undefined> | undefined>(undefined)
  const attachmentRef = useRef<PanelConversationAttachment | undefined>(
    undefined,
  )
  const localStreamConversationRef = useRef<string | undefined>(undefined)
  const localStreamRunRef = useRef<string | undefined>(undefined)
  const streamRequestRef = useRef<Promise<void> | undefined>(undefined)
  const viewTransitionRef = useRef<Promise<void>>(Promise.resolve())
  const owningTab = () => {
    panelTabRef.current ??= resolvePanelTabId()
    return panelTabRef.current
  }

  useEffect(() => {
    optionsRef.current = options
  }, [options])

  useEffect(() => {
    conversationIdRef.current = conversationId
  }, [conversationId])

  const {
    startTask: startExecutionTask,
    syncFromMessages: syncExecutionHistory,
    finishTask: finishExecutionTask,
  } = useExecutionHistoryTracker({ enabled: persistHistory })

  const onClickLike = (messageId: string) => {
    const { responseText, queryText } = getResponseAndQueryFromMessageId(
      messages,
      messageId,
    )

    track(MESSAGE_LIKE_EVENT, { responseText, queryText, messageId })

    setLiked((prev) => ({
      ...prev,
      [messageId]: !prev[messageId],
    }))
  }

  const onClickDislike = (messageId: string, comment?: string) => {
    const { responseText, queryText } = getResponseAndQueryFromMessageId(
      messages,
      messageId,
    )

    track(MESSAGE_DISLIKE_EVENT, {
      responseText,
      queryText,
      messageId,
      comment,
    })

    setDisliked((prev) => ({
      ...prev,
      [messageId]: !prev[messageId],
    }))
  }

  const modeRef = useRef<ChatMode>(mode)
  const textToActionRef = useRef<Map<string, ChatAction>>(textToAction)
  const workingDirRef = useRef<string | undefined>(undefined)
  const selectionMapRef = useRef<
    Record<string, { text: string; url: string; title: string }>
  >({})
  const pendingSelectionTabKeyRef = useRef<string | null>(null)
  const messagesRef = useRef<UIMessage[]>([])

  useEffect(() => {
    const toRef = (
      map: Record<string, { text: string; pageUrl: string; pageTitle: string }>,
    ) => {
      const result: Record<
        string,
        { text: string; url: string; title: string }
      > = {}
      for (const [k, v] of Object.entries(map)) {
        result[k] = { text: v.text, url: v.pageUrl, title: v.pageTitle }
      }
      return result
    }
    selectedTextStorage.getValue().then((map) => {
      selectionMapRef.current = toRef(map)
    })
    const unwatchText = selectedTextStorage.watch((map) => {
      selectionMapRef.current = toRef(map)
    })
    return () => unwatchText()
  }, [])

  useEffect(() => {
    selectedWorkspaceStorage.getValue().then((folder) => {
      workingDirRef.current = folder?.path
    })

    const unwatch = selectedWorkspaceStorage.watch((folder) => {
      workingDirRef.current = folder?.path
    })
    return () => unwatch()
  }, [])

  useDeepCompareEffect(() => {
    modeRef.current = mode
    textToActionRef.current = textToAction
  }, [mode, textToAction])

  const selectedProvider = selectedChatTarget
    ? toProviderOption(selectedChatTarget)
    : providers[0]

  const transportRef = useRef<DefaultChatTransport<UIMessage> | null>(null)
  if (!transportRef.current) {
    transportRef.current = new DefaultChatTransport<UIMessage>({
      prepareReconnectToStreamRequest: async ({ body }) => {
        const serverUrl = await resolveAgentServerUrl()
        return {
          api: conversationReconnectUrl(
            serverUrl,
            typeof body?.conversationId === 'string'
              ? body.conversationId
              : conversationIdRef.current,
            typeof body?.runId === 'string' ? body.runId : undefined,
          ),
        }
      },
      prepareSendMessagesRequest: async ({ messages }) => {
        const target = selectedChatTargetRef.current
        // No fabricated fallback. Sending a provider the user never configured
        // would run the turn on credentials they did not choose; sending
        // nothing lets the server resolve its own selection, or say there is
        // none.
        const fallbackProvider =
          resolveChatProvider(
            selectedLlmProviderRef.current
              ? [selectedLlmProviderRef.current]
              : [],
          ) ?? undefined
        // A contextual panel sends from its owning tab even if another tab
        // becomes active while provider/server preparation is awaiting I/O.
        const tabId =
          optionsRef.current?.origin === 'newtab'
            ? undefined
            : await owningTab()
        const activeTab =
          tabId !== undefined
            ? await chrome.tabs.get(tabId)
            : (
                await chrome.tabs.query({ active: true, currentWindow: true })
              )[0]
        const activeTabSelection = activeTab?.id
          ? (selectionMapRef.current[String(activeTab.id)] ?? null)
          : null
        const currentMode = modeRef.current
        const enabledMcpServers = enabledMcpServersRef.current
        const customMcpServers = enabledCustomServersRef.current
        const lastUserMessage = getLastUserMessageText(messages)
        const action = textToActionRef.current.get(lastUserMessage)
        const requestBrowserContext = buildRequestBrowserContext({
          activeTab,
          action,
          enabledMcpServers,
          customMcpServers,
        })

        const declinedApps = await declinedAppsStorage.getValue()
        const persist = persistRef.current
        const previousMessages = messagesRef.current
        // When the server persists, it owns history and loads it itself, so
        // the client stops replaying it. Only an unpersisted conversation has
        // to ship its own projection.
        const history =
          !persist && previousMessages.length > 0
            ? formatConversationHistory(previousMessages)
            : undefined
        const previousConversation = history?.length ? history : undefined

        const userSystemPrompt = getUserSystemPrompt(
          optionsRef.current?.origin,
          personalizationRef.current,
        )
        const commonRequest = {
          conversationId: conversationIdRef.current,
          mode: currentMode,
          browserContext: requestBrowserContext,
          userSystemPrompt,
          userWorkingDir: workingDirRef.current,
          previousConversation,
          persist,
          declinedApps,
          attachments: getLastUserMessageFiles(messages).map((file) => ({
            mediaType: file.mediaType,
            data: file.url,
          })),
        }

        const message = getLastMessageText(messages)

        const result = await prepareSidepanelSendMessagesRequest({
          resolveAgentServerUrl,
          target,
          fallbackProvider,
          message,
          ...commonRequest,
          selectedText: activeTabSelection?.text,
          selectedTextSource: activeTabSelection
            ? {
                url: activeTabSelection.url,
                title: activeTabSelection.title,
              }
            : undefined,
        })

        // Track which tab's selection was sent so we can clear it on success
        pendingSelectionTabKeyRef.current =
          activeTabSelection && activeTab?.id ? String(activeTab.id) : null

        return result
      },
    })
  }

  const chatTransport = transportRef.current

  const {
    messages,
    sendMessage: baseSendMessage,
    setMessages,
    status,
    stop: detachStream,
    resumeStream,
    error: chatError,
    regenerate,
  } = useChat({
    transport: chatTransport,
    onError: () => {
      // A failed local POST belongs to the user's Retry action. Only mirrored
      // stream failures should rehydrate; replaying an older run here would
      // erase the failed prompt and hide the provider error in the source panel.
      if (!localStreamConversationRef.current) attachmentRef.current?.retry()
    },
    onFinish: async ({ message, messages, isAbort, isError, finishReason }) => {
      const nextMessages = addContentFilterNotice(
        messages,
        message,
        finishReason,
      )
      if (nextMessages !== messages) {
        setMessages(nextMessages)
      }
      const responseMessage =
        nextMessages.find((each) => each.id === message.id) ?? message
      await finishExecutionTask({
        responseText: getLastMessageText([responseMessage]),
        isAbort,
        isError,
      })
    },
  })

  const detachView = useCallback(async () => {
    await detachStream()
    // SDK stop signals cancellation immediately; await the request's unwind
    // before installing another transcript so old onFinish work cannot mutate it.
    await streamRequestRef.current?.catch(() => undefined)
  }, [detachStream])

  useEffect(() => {
    if (options?.origin !== 'newtab') return
    // Leaving a routed chat detaches its view; the server still owns the run.
    return () => {
      void detachView()
    }
  }, [detachView, options?.origin])

  const stop = useCallback(async () => {
    // First detach this view so the UI responds immediately, then cancel the
    // server-owned run explicitly. Aborting the fetch alone is intentionally
    // no longer a lifecycle signal.
    const stoppedConversationId = conversationIdRef.current
    const detaching = detachView()
    try {
      const serverUrl = agentUrlRef.current ?? (await resolveAgentServerUrl())
      const response = await fetch(
        `${serverUrl}/chat/${encodeURIComponent(stoppedConversationId)}/stop`,
        { method: 'POST' },
      )
      if (!response.ok) {
        throw new Error(`Conversation stop failed (${response.status})`)
      }
    } catch (error) {
      sentry.captureException(error, {
        extra: {
          conversationId: conversationIdRef.current,
          operation: 'stop-server-conversation',
        },
      })
    } finally {
      await detaching
    }
  }, [detachView])

  useEffect(() => {
    if (optionsRef.current?.origin === 'newtab') return
    let cancelled = false
    let tabId: number | undefined
    let latestViews:
      | Awaited<ReturnType<typeof conversationPanelViewsStorage.getValue>>
      | undefined
    const attachment = new PanelConversationAttachment({
      load: async (id, signal) =>
        fetchConversationRunState(
          agentUrlRef.current ?? (await resolveAgentServerUrl()),
          id,
          fetch,
          AbortSignal.any([signal, AbortSignal.timeout(10_000)]),
        ),
      ownsLocalStream: (id, runId) => {
        if (localStreamConversationRef.current !== id) return false
        localStreamRunRef.current ??= runId
        return localStreamRunRef.current === runId
      },
      attach: async (state, isCurrent) => {
        await detachView()
        if (!isCurrent()) return
        const id = state.conversationId as ReturnType<typeof crypto.randomUUID>
        conversationIdRef.current = id
        messagesRef.current = state.messages
        setConversationId(id)
        setMessages(state.messages)
        setSearchParamsRef.current({}, { replace: true })
        if (state.status === 'running') {
          streamRequestRef.current = resumeStream({
            body: {
              conversationId: state.conversationId,
              runId: state.runId,
            },
          })
        }
      },
      clear: (id) => {
        if (conversationIdRef.current !== id) return
        const nextId = crypto.randomUUID()
        conversationIdRef.current = nextId
        viewTransitionRef.current = detachView().then(() => {
          if (conversationIdRef.current !== nextId) return
          messagesRef.current = []
          setMessages([])
          setConversationId(nextId)
        })
      },
      reportError: (error) =>
        sentry.captureException(error, {
          extra: { operation: 'attach-panel-conversation' },
        }),
    })
    attachmentRef.current = attachment
    const unwatch = conversationPanelViewsStorage.watch((views) => {
      latestViews = views
      if (tabId !== undefined)
        attachment.update(conversationForTab(views, tabId))
    })
    void (async () => {
      panelTabRef.current ??= resolvePanelTabId()
      tabId = await panelTabRef.current
      const views = await conversationPanelViewsStorage.getValue()
      if (!cancelled)
        attachment.update(conversationForTab(latestViews ?? views, tabId))
    })().catch((error) => sentry.captureException(error))
    return () => {
      cancelled = true
      attachment.dispose()
      if (attachmentRef.current === attachment)
        attachmentRef.current = undefined
      unwatch()
      void detachView()
    }
  }, [detachView, resumeStream, setMessages])

  // Two cleanups once a turn is no longer streaming: drop messages with
  // empty parts (interrupted responses trip AI SDK validation on the next
  // send), and strip retained base64 image tool outputs from older turns.
  // Nothing renders those screenshots, but the AI SDK keeps every message
  // resident, so they accumulate until the renderer OOMs (#1972). The latest
  // message stays intact so the just-finished turn is untouched.
  useEffect(() => {
    if (status === 'streaming') return
    const nonEmpty = messages.some((m) => !m.parts?.length)
      ? messages.filter((m) => m.parts?.length > 0)
      : messages
    const cleaned = stripImageToolOutputs(nonEmpty, { keepLastMessage: true })
    if (cleaned !== messages) setMessages(cleaned)
  }, [messages, status, setMessages])

  // The URL is retained in new tabs for refresh/back navigation. Its keyed
  // provider isolates each selection; sidepanel keeps its existing transient URL.
  // biome-ignore lint/correctness/useExhaustiveDependencies: target selection changes during restore; only restart for route/load/retry changes
  useEffect(() => {
    if (!conversationIdParam) return
    if (restoredConversationId === conversationIdParam) return

    if (isLoadingProviders || isLoadingAgentUrl) return
    let cancelled = false
    setRestoreError(null)
    void restoreServerConversation({
      conversationId: conversationIdParam,
      fetchConversation: async (id) => {
        await detachView()
        return fetchServerConversation(id)
      },
      isCancelled: () => cancelled,
      onRestore: async (conversation) => {
        // Select before enabling the composer. ACP sessions are keyed by both
        // conversation and agent, so a different agent would lose their context.
        const target = resolveRestoredChatTarget(
          conversation,
          chatTargets,
          selectedChatTargetRef.current,
        )
        if (
          options?.origin === 'newtab' &&
          target &&
          (target.id !== selectedChatTargetRef.current?.id ||
            target.kind !== selectedChatTargetRef.current?.kind)
        ) {
          await selectChatTarget(target)
        }
        if (cancelled) return
        const id = conversation.id as ReturnType<typeof crypto.randomUUID>
        conversationIdRef.current = id
        messagesRef.current = conversation.messages
        setConversationId(id)
        setMessages(conversation.messages)
        if (options?.origin === 'newtab' && !target)
          setRestoreError(
            'The agent used for this conversation is no longer available. Restore it in Settings to continue.',
          )
      },
      onMissing: () => {
        if (options?.origin === 'newtab')
          setRestoreError(
            'This conversation is no longer available. Choose another conversation or start a new one.',
          )
      },
      onError: (error) => {
        if (options?.origin === 'newtab')
          setRestoreError('Couldn’t open this conversation. Please try again.')
        sentry.captureException(error, {
          extra: { conversationId: conversationIdParam },
        })
      },
      onSettled: () => {
        setRestoredConversationId(conversationIdParam)
        if (options?.origin !== 'newtab') setSearchParams({}, { replace: true })
      },
    })
    return () => {
      cancelled = true
    }
  }, [
    conversationIdParam,
    isLoadingProviders,
    isLoadingAgentUrl,
    restoreAttempt,
  ])

  // Keep messagesRef in sync on every change (cheap ref assignment)
  useEffect(() => {
    messagesRef.current = messages
    syncExecutionHistory(messages, status)
  }, [messages, status, syncExecutionHistory])

  // Save conversation only after a turn terminates — not on every token
  const previousStatusRef = useRef(status)
  // biome-ignore lint/correctness/useExhaustiveDependencies: only save when a turn terminates
  useEffect(() => {
    const justFinished = didStreamingTurnFinish(
      previousStatusRef.current,
      status,
    )
    previousStatusRef.current = status

    if (!justFinished) return

    // Clear the selected text that was sent with this request
    const tabKey = pendingSelectionTabKeyRef.current
    if (status === 'ready' && tabKey) {
      pendingSelectionTabKeyRef.current = null
      delete selectionMapRef.current[tabKey]
      selectedTextStorage.getValue().then((map) => {
        if (map[tabKey]) {
          const { [tabKey]: _, ...rest } = map
          selectedTextStorage.setValue(rest)
        }
      })
    }

    const messagesToSave = getPersistableMessages(messages)
    if (messagesToSave.length === 0) return

    // The local server persists every turn during /chat, so the client has no
    // history write of its own left. Incognito still writes nowhere (#1189).
    if (persistHistory)
      void queryClient.invalidateQueries({
        queryKey: [SERVER_CONVERSATIONS_QUERY_KEY],
      })
  }, [status])

  // Save the in-flight conversation before it can be lost: on page hide (full
  // navigation, tab switch, close) and on unmount, because an in-app SPA route
  // The durable turn buffer and its flush lived here to survive an
  // interrupted cloud upload. The local server persists each turn during
  // /chat, so there is nothing left to buffer.

  const isIntegrationsSynced = options?.isIntegrationsSynced ?? true
  const isIntegrationsSyncedRef = useRef(isIntegrationsSynced)
  /**
   * Sends held while something was still loading, in the order they were made.
   *
   * A list rather than one slot. A send that lands here clears the composer,
   * so a second one during the same wait used to replace the first and the
   * first was never dispatched: the user watched their message disappear.
   */
  const pendingMessagesRef = useRef<
    Array<{
      text: string
      action?: ChatAction
      files?: FileUIPart[]
    }>
  >([])

  const trackMessageSent = useCallback(() => {
    const target = selectedChatTargetRef.current
    const llmTargetProvider = toLlmProviderConfig(target)
    const agentTarget = target?.kind === 'acp' ? target : undefined
    track(MESSAGE_SENT_EVENT, {
      mode,
      provider_id:
        agentTarget?.agentId ??
        llmTargetProvider?.id ??
        selectedLlmProvider?.id,
      provider_type: agentTarget ? 'acp' : llmTargetProvider?.type,
      agent_id: agentTarget?.agentId,
      adapter: agentTarget?.agentType,
      model:
        agentTarget?.modelId ??
        llmTargetProvider?.modelId ??
        selectedLlmProvider?.modelId,
    })
  }, [mode, selectedChatTargetRef, selectedLlmProvider])

  // Both Send and Retry own POST streams. Adopt their assignment once without
  // interrupting the request; later runs from another panel still hydrate.
  const runLocalRequest = useCallback((request: () => Promise<void>) => {
    const id = conversationIdRef.current
    return viewTransitionRef.current.then(async () => {
      if (conversationIdRef.current !== id) return
      localStreamConversationRef.current = id
      localStreamRunRef.current = undefined
      attachmentRef.current?.beginLocalTurn(id)
      const pending = request()
      streamRequestRef.current = pending
      try {
        await pending
      } finally {
        if (streamRequestRef.current === pending)
          localStreamConversationRef.current = undefined
      }
    })
  }, [])

  const dispatchMessage = useCallback(
    (text: string, files?: FileUIPart[]) => {
      // Returns the turn's promise so a caller sending several can wait for
      // each before starting the next.
      return runLocalRequest(() => {
        trackMessageSent()
        startExecutionTask({
          conversationId: conversationIdRef.current,
          promptText: text,
        })
        return baseSendMessage({ text, files })
      }).catch((error) => sentry.captureException(error))
    },
    [baseSendMessage, runLocalRequest, startExecutionTask, trackMessageSent],
  )

  useEffect(() => {
    isIntegrationsSyncedRef.current = isIntegrationsSynced
  }, [isIntegrationsSynced])

  // Flushes what `sendMessage` held while something was still loading. It waits
  // on the target lists too: a queued message dispatched before they settle
  // would name no provider and come back as a server-side rejection.
  useEffect(() => {
    if (!isSettled || pendingMessagesRef.current.length === 0) return

    if (!hasAnyTarget) {
      // The wait resolved to nothing connected. Say so, and keep the messages:
      // connecting a provider re-runs this and sends them, so a handoff whose
      // query parameters are already gone is not lost.
      setSendAttemptBlocked(true)
      return
    }

    if (!isIntegrationsSynced || !agentServerUrl) return

    // Taken before the first await so a re-run cannot dispatch them twice.
    const queued = pendingMessagesRef.current
    pendingMessagesRef.current = []

    void drainPendingSends(queued, (pending) => {
      const { action } = pending
      if (action) {
        setTextToAction((prev) => {
          const next = new Map(prev)
          next.set(pending.text, action)
          return next
        })
      }
      return dispatchMessage(pending.text, pending.files)
    })
  }, [
    agentServerUrl,
    dispatchMessage,
    hasAnyTarget,
    isIntegrationsSynced,
    isSettled,
  ])

  /**
   * Sends, or reports why it did not.
   *
   * The boolean matters: callers clear the composer after this returns, so a
   * refusal that looked like a send would take the user's draft with it. Every
   * chat surface funnels through here, so the no-provider guard only has to
   * exist once.
   */
  const sendMessage = (params: {
    text: string
    action?: ChatAction
    files?: FileUIPart[]
  }): boolean => {
    const decision = decideChatSend({
      isRestoring: isRestoringConversation,
      hasRestoreError: Boolean(restoreError),
      isSettled: isSettledRef.current,
      hasAnyTarget: hasAnyTargetRef.current,
      isIntegrationsSynced: isIntegrationsSyncedRef.current,
      hasAgentUrl: Boolean(agentUrlRef.current),
    })

    if (decision === 'drop') return false
    if (decision === 'refuse') {
      setSendAttemptBlocked(true)
      return false
    }
    if (decision === 'queue') {
      // Retained, not refused: it still reaches the model once the wait is
      // over, so the caller is right to clear the composer.
      pendingMessagesRef.current.push(params)
      return true
    }

    if (params.action) {
      const action = params.action
      setTextToAction((prev) => {
        const next = new Map(prev)
        next.set(params.text, action)
        return next
      })
    }
    dispatchMessage(params.text, params.files)
    return true
  }

  // biome-ignore lint/correctness/useExhaustiveDependencies: only need to run this once
  useEffect(() => {
    const unwatch = searchActionsStorage.watch((storageAction) => {
      if (storageAction) {
        setMode(storageAction.mode)
        sendMessage({ text: storageAction.query, action: storageAction.action })
      }
    })
    return () => unwatch()
  }, [])

  // biome-ignore lint/correctness/useExhaustiveDependencies: only need to run this once
  useEffect(() => {
    const unwatch = stopAgentStorage.watch((signal) => {
      if (signal && signal.conversationId === conversationIdRef.current) {
        stop()
        track(GLOW_STOP_CLICKED_EVENT)
        stopAgentStorage.setValue(null)
      }
    })
    return () => unwatch()
  }, [])

  const discardServerSession = useCallback((conversationId: string) => {
    const serverUrl = agentUrlRef.current
    if (!serverUrl) return
    void fetch(`${serverUrl}/chat/${encodeURIComponent(conversationId)}`, {
      method: 'DELETE',
      keepalive: true,
    })
      .then((response) => {
        if (!response.ok && response.status !== 404) {
          throw new Error(`Session cleanup failed (${response.status})`)
        }
      })
      .catch((error) => {
        sentry.captureException(error, {
          extra: { conversationId },
        })
      })
  }, [])

  const resetConversationState = () => {
    const previousConversationId = conversationIdRef.current
    attachmentRef.current?.retire(previousConversationId)
    pendingMessagesRef.current = []
    localStreamConversationRef.current = undefined
    discardServerSession(previousConversationId)
    const nextId = crypto.randomUUID()
    conversationIdRef.current = nextId
    viewTransitionRef.current = detachView().then(() => {
      if (conversationIdRef.current !== nextId) return
      messagesRef.current = []
      setConversationId(nextId)
      setMessages([])
    })
    setTextToAction(new Map())
    setLiked({})
    setDisliked({})
    setRestoredConversationId(null)
    setRestoreError(null)
    // Clearing the restore param also cancels any in-flight logged-out restore
    // (via the restore effect's cleanup), so a stale response can't revive the
    // old conversation over this new blank session.
    setSearchParams({}, { replace: true })
  }

  const handleSelectProvider = (provider: Provider) => {
    const target = chatTargets.find(
      (candidate) =>
        candidate.id === provider.id && candidate.kind === provider.kind,
    )
    if (!target) return

    const previousTarget = selectedChatTargetRef.current
    track(PROVIDER_SELECTED_EVENT, {
      provider_id: target.id,
      provider_type: target.kind === 'acp' ? 'acp' : target.type,
      model_id:
        target.kind === 'acp' ? target.modelId : target.provider.modelId,
      agent_id: target.kind === 'acp' ? target.agentId : undefined,
      adapter: target.kind === 'acp' ? target.agentType : undefined,
    })

    void selectChatTarget(target).catch((error) => {
      sentry.captureException(error, {
        extra: {
          message: 'Failed to persist sidepanel chat target selection',
          targetId: target.id,
          targetKind: target.kind,
        },
      })
    })

    if (
      previousTarget &&
      (previousTarget.kind !== target.kind ||
        previousTarget.id !== target.id) &&
      messagesRef.current.length > 0
    ) {
      resetConversationState()
    }
  }

  const getActionForMessage = (message: UIMessage) => {
    if (message.role !== 'user') return undefined
    const text = message.parts
      .filter((part) => part.type === 'text')
      .map((part) => part.text)
      .join('')
    return textToAction.get(text)
  }

  const resetConversation = () => {
    track(CONVERSATION_RESET_EVENT, { message_count: messages.length })
    resetConversationState()
  }

  return {
    mode,
    setMode,
    messages,
    sendMessage,
    status,
    stop,
    providers,
    selectedProvider,
    isLoading: isLoadingProviders || isLoadingAgentUrl,
    hasAnyTarget,
    isSettled,
    sendBlocked,
    canSend,
    isSyncing: !isIntegrationsSynced,
    isIncognito,
    isRestoringConversation,
    restoreError,
    retryRestoreConversation: () => {
      setRestoredConversationId(null)
      setRestoreAttempt((value) => value + 1)
    },
    agentUrlError,
    chatError,
    retryLastTurn: () => runLocalRequest(() => regenerate()),
    handleSelectProvider,
    getActionForMessage,
    resetConversation,
    liked,
    onClickLike,
    disliked,
    onClickDislike,
    conversationId,
  }
}
