import { useChat } from '@ai-sdk/react'
import { DefaultChatTransport, type FileUIPart, type UIMessage } from 'ai'
import { compact } from 'es-toolkit/array'
import { useCallback, useEffect, useRef, useState } from 'react'
import { useSearchParams } from 'react-router'
import useDeepCompareEffect from 'use-deep-compare-effect'
import type { Provider } from '@/components/chat/chatComponentTypes'
import { isIncognitoWindow } from '@/lib/browseros/incognito'
import {
  getWindowConversation,
  setWindowConversation,
} from '@/lib/browseros/perWindowConversationStorage'
import { sidePanelPerWindowStorage } from '@/lib/browseros/sidePanelOpenStateStorage'
import type { ChatAction } from '@/lib/chat-actions/types'
import {
  CONVERSATION_RESET_EVENT,
  GLOW_STOP_CLICKED_EVENT,
  MESSAGE_DISLIKE_EVENT,
  MESSAGE_LIKE_EVENT,
  MESSAGE_SENT_EVENT,
  PROVIDER_SELECTED_EVENT,
} from '@/lib/constants/analyticsEvents'
import {
  bufferActiveConversation,
  flushActiveConversationBuffer,
} from '@/lib/conversations/active-conversation-buffer'
import { formatConversationHistory } from '@/lib/conversations/formatConversationHistory'
import { uploadConversations } from '@/lib/conversations/uploadConversationsToGraphql'
import { declinedAppsStorage } from '@/lib/declined-apps/storage'
import { resolveChatProvider } from '@/lib/llm-providers/provider-runtime'
import { createDefaultBrowserOSProvider } from '@/lib/llm-providers/storage'
import type { ChatRequestBrowserContext } from '@/lib/messaging/server/buildChatRequestBody'
import { track } from '@/lib/metrics/track'
import { searchActionsStorage } from '@/lib/search-actions/searchActionsStorage'
import { selectedTextStorage } from '@/lib/selected-text/selectedTextStorage'
import { sentry } from '@/lib/sentry/sentry'
import { stopAgentStorage } from '@/lib/stop-agent/stop-agent-storage'
import { selectedWorkspaceStorage } from '@/lib/workspace/workspace-storage'
import { resolveAgentServerUrlWithRetry } from '@/modules/browseros/agent-server-url.helpers'
import { useAgentServerUrl } from '@/modules/browseros/agent-server-url.hooks'
import { fetchServerConversation } from '@/modules/conversations/conversations.hooks'
import { useInvalidateCredits } from '@/modules/credits/credits.hooks'
import { useGraphqlQuery } from '@/modules/graphql/graphql-query.hooks'
import { useChatRefs } from './chat-refs.hooks'
import { GetConversationWithMessagesDocument } from './chat-session-document'
import {
  didStreamingTurnFinish,
  getPersistableMessages,
  shouldPersistHistory,
} from './chat-session-persistence'
import {
  prepareSidepanelSendMessagesRequest,
  toProviderOption,
} from './chat-session-request'
import { restoreServerConversation } from './chat-session-restore'
import type { ChatMode } from './chat-types'
import { addContentFilterNotice } from './content-filter-notice'
import { useExecutionHistoryTracker } from './execution-history-tracker.hooks'
import { useNotifyActiveTab } from './notify-active-tab.hooks'
import { useRemoteConversationSave } from './remote-conversation-save.hooks'
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
  } = useChatRefs()
  const invalidateCredits = useInvalidateCredits()

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

  const {
    isLoggedIn,
    userId,
    saveConversation: saveRemoteConversation,
    resetConversation: resetRemoteConversation,
    markMessagesAsSaved,
  } = useRemoteConversationSave()
  const [searchParams, setSearchParams] = useSearchParams()
  const conversationIdParam = searchParams.get('conversationId')

  // 'local': the local server owns history (persisted during /chat, loaded from
  // SQLite); 'cloud': the client owns it (logged-in cloud sync, or incognito).
  // Read via a ref because the transport closure below is created only once.
  const historyModeRef = useRef<'local' | 'cloud'>('cloud')
  useEffect(() => {
    historyModeRef.current = !isLoggedIn && persistHistory ? 'local' : 'cloud'
  }, [isLoggedIn, persistHistory])

  const agentUrlRef = useRef(agentServerUrl)

  useEffect(() => {
    agentUrlRef.current = agentServerUrl
  }, [agentServerUrl])

  const canSend = !isLoadingAgentUrl && !agentUrlError && !!agentServerUrl

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
  // The window this panel belongs to, resolved on mount in per-window scope.
  const windowIdRef = useRef<number | null>(null)

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
      prepareSendMessagesRequest: async ({ messages }) => {
        const target = selectedChatTargetRef.current
        const fallbackProvider =
          resolveChatProvider(
            selectedLlmProviderRef.current
              ? [selectedLlmProviderRef.current]
              : [],
          ) ?? createDefaultBrowserOSProvider()
        const activeTabsList = await chrome.tabs.query({
          active: true,
          currentWindow: true,
        })
        const activeTab = activeTabsList?.[0] ?? undefined
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
        const historyMode = historyModeRef.current
        const previousMessages = messagesRef.current
        // In local mode the server owns history and loads it from SQLite, so
        // the client stops replaying it. Cloud mode still ships the projection.
        const history =
          historyMode === 'cloud' && previousMessages.length > 0
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
          historyMode,
          declinedApps,
          attachments: getLastUserMessageFiles(messages).map((file) => ({
            mediaType: file.mediaType,
            data: file.url,
          })),
        }

        const message = getLastMessageText(messages)

        const result = await prepareSidepanelSendMessagesRequest({
          resolveAgentServerUrl: resolveAgentServerUrlWithRetry,
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
    stop,
    error: chatError,
    regenerate,
  } = useChat({
    transport: chatTransport,
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

  useNotifyActiveTab({
    messages,
    status,
    conversationId: conversationIdRef.current,
  })

  const {
    data: remoteConversationData,
    isFetched: isRemoteConversationFetched,
  } = useGraphqlQuery(
    GetConversationWithMessagesDocument,
    { conversationId: conversationIdParam ?? '' },
    {
      enabled: !!conversationIdParam && isLoggedIn,
    },
  )

  const [restoredConversationId, setRestoredConversationId] = useState<
    string | null
  >(null)

  // biome-ignore lint/correctness/useExhaustiveDependencies: restore should only run when query data arrives or conversationIdParam changes
  useEffect(() => {
    if (!conversationIdParam) return
    if (restoredConversationId === conversationIdParam) return

    if (isLoggedIn) {
      if (!isRemoteConversationFetched) return

      if (remoteConversationData?.conversation) {
        const restoredMessages =
          remoteConversationData.conversation.conversationMessages.nodes
            .filter((node): node is NonNullable<typeof node> => node !== null)
            .map((node) => node.message as UIMessage)

        setConversationId(
          conversationIdParam as ReturnType<typeof crypto.randomUUID>,
        )
        setMessages(restoredMessages)
        markMessagesAsSaved(conversationIdParam, restoredMessages)
      }
      setRestoredConversationId(conversationIdParam)
      setSearchParams({}, { replace: true })
      return
    }

    let cancelled = false
    void restoreServerConversation({
      conversationId: conversationIdParam,
      fetchConversation: fetchServerConversation,
      isCancelled: () => cancelled,
      onRestore: (conversation) => {
        setConversationId(
          conversation.id as ReturnType<typeof crypto.randomUUID>,
        )
        setMessages(conversation.messages)
      },
      onError: (error) =>
        sentry.captureException(error, {
          extra: { conversationId: conversationIdParam },
        }),
      onSettled: () => {
        setRestoredConversationId(conversationIdParam)
        setSearchParams({}, { replace: true })
      },
    })
    return () => {
      cancelled = true
    }
  }, [conversationIdParam, remoteConversationData, isLoggedIn])

  // Per-window scope: resume this window's conversation when the panel
  // (re)mounts (e.g. closed + reopened) instead of starting a blank chat.
  // No-op in per-tab scope. Tab switches keep the same panel instance, so this
  // only matters for a fresh mount.
  // biome-ignore lint/correctness/useExhaustiveDependencies: mount-only; reads refs
  useEffect(() => {
    let cancelled = false
    ;(async () => {
      if (!(await sidePanelPerWindowStorage.getValue())) return
      const [tab] = await chrome.tabs.query({
        active: true,
        currentWindow: true,
      })
      const windowId = tab?.windowId
      if (windowId == null || cancelled) return
      windowIdRef.current = windowId
      const stored = await getWindowConversation(windowId)
      if (cancelled) return
      if (stored && stored !== conversationIdRef.current) {
        setSearchParams({ conversationId: stored })
      } else if (!stored) {
        await setWindowConversation(windowId, conversationIdRef.current)
      }
    })()
    return () => {
      cancelled = true
    }
  }, [])

  // Remember the conversation this window is on so a remount can resume it.
  useEffect(() => {
    const windowId = windowIdRef.current
    if (windowId == null) return
    ;(async () => {
      if (!(await sidePanelPerWindowStorage.getValue())) return
      await setWindowConversation(windowId, conversationId)
    })()
  }, [conversationId])

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

    // Skip all history writes in incognito so the chat never becomes durable
    // (neither local nor cloud) and can't surface in a normal window (#1189).
    // Logged-out history is owned by the local server (persisted during /chat
    // in local mode), so only the cloud lane writes from the client now.
    if (persistHistory && isLoggedIn) {
      // Buffer the settled turn durably before the fire-and-forget cloud
      // write, so an interrupted navigation still lets the next mount sync it
      // (#559).
      if (userId) {
        void bufferActiveConversation({
          id: conversationIdRef.current,
          messages: messagesToSave,
          lastMessagedAt: Date.now(),
          userId,
        })
      }
      saveRemoteConversation(conversationIdRef.current, messagesToSave)
    }

    invalidateCredits()
  }, [status])

  // Save the in-flight conversation before it can be lost: on page hide (full
  // navigation, tab switch, close) and on unmount, because an in-app SPA route
  // change to Settings unmounts the chat while the page stays visible, so
  // visibilitychange never fires. Reads the latest messages either way; the next
  // mount then syncs it to the cloud (#559). The settled turn is also buffered
  // at turn end above. This effect's deps are the auth pair, not messages, so
  // the unmount write runs once, not on every token.
  useEffect(() => {
    if (!persistHistory || !isLoggedIn || !userId) return
    const writeBuffer = () => {
      const latest = getPersistableMessages(messagesRef.current)
      if (latest.length === 0) return
      void bufferActiveConversation({
        id: conversationIdRef.current,
        messages: latest,
        lastMessagedAt: Date.now(),
        userId,
      })
    }
    const onHide = () => {
      if (document.visibilityState === 'hidden') writeBuffer()
    }
    document.addEventListener('visibilitychange', onHide)
    return () => {
      document.removeEventListener('visibilitychange', onHide)
      writeBuffer()
    }
  }, [persistHistory, isLoggedIn, userId])

  // On mount (and on sign-in), push any buffered in-flight conversations for the
  // current user to the cloud so an interrupted chat still lands in history. It
  // is never restored into the active conversation; recovery is via history.
  useEffect(() => {
    if (!persistHistory || !isLoggedIn || !userId) return
    void flushActiveConversationBuffer(userId, (conversations) =>
      uploadConversations(conversations, userId),
    )
  }, [persistHistory, isLoggedIn, userId])

  useEffect(() => {
    if (chatError) invalidateCredits()
  }, [chatError, invalidateCredits])

  const isIntegrationsSynced = options?.isIntegrationsSynced ?? true
  const isIntegrationsSyncedRef = useRef(isIntegrationsSynced)
  const pendingMessageRef = useRef<{
    text: string
    action?: ChatAction
    files?: FileUIPart[]
  } | null>(null)

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

  const dispatchMessage = useCallback(
    (text: string, files?: FileUIPart[]) => {
      trackMessageSent()
      startExecutionTask({
        conversationId: conversationIdRef.current,
        promptText: text,
      })
      baseSendMessage({ text, files })
    },
    [baseSendMessage, startExecutionTask, trackMessageSent],
  )

  useEffect(() => {
    isIntegrationsSyncedRef.current = isIntegrationsSynced
  }, [isIntegrationsSynced])

  useEffect(() => {
    if (isIntegrationsSynced && agentServerUrl && pendingMessageRef.current) {
      const pending = pendingMessageRef.current
      pendingMessageRef.current = null
      const { action } = pending
      if (action) {
        setTextToAction((prev) => {
          const next = new Map(prev)
          next.set(pending.text, action)
          return next
        })
      }
      dispatchMessage(pending.text, pending.files)
    }
  }, [agentServerUrl, dispatchMessage, isIntegrationsSynced])

  const sendMessage = (params: {
    text: string
    action?: ChatAction
    files?: FileUIPart[]
  }) => {
    if (!isIntegrationsSyncedRef.current || !agentUrlRef.current) {
      pendingMessageRef.current = params
      return
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

  useEffect(
    () => () => discardServerSession(conversationIdRef.current),
    [discardServerSession],
  )

  const resetConversationState = () => {
    const previousConversationId = conversationIdRef.current
    stop()
    void finishExecutionTask({ isAbort: true })
    discardServerSession(previousConversationId)
    setConversationId(crypto.randomUUID())
    setMessages([])
    setTextToAction(new Map())
    setLiked({})
    setDisliked({})
    setRestoredConversationId(null)
    // Clearing the restore param also cancels any in-flight logged-out restore
    // (via the restore effect's cleanup), so a stale response can't revive the
    // old conversation over this new blank session.
    setSearchParams({}, { replace: true })
    resetRemoteConversation()
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

  const isRestoringConversation =
    !!conversationIdParam && restoredConversationId !== conversationIdParam

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
    canSend,
    isSyncing: !isIntegrationsSynced,
    isIncognito,
    isRestoringConversation,
    agentUrlError,
    chatError,
    retryLastTurn: regenerate,
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
