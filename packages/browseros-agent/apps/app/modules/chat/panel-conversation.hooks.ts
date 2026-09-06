import { useCallback, useEffect, useRef, useState } from 'react'
import { useSearchParams } from 'react-router'
import {
  type ConversationPanelView,
  conversationPanelViewsStorage,
} from '@/lib/browseros/conversationPanelStorage'
import {
  getWindowConversation,
  setWindowConversation,
} from '@/lib/browseros/perWindowConversationStorage'
import {
  RuntimeMessageType,
  sendRuntimeMessage,
} from '@/lib/messaging/runtime/runtimeMessages'
import { sentry } from '@/lib/sentry/sentry'
import { panelTabIdFromUrl } from './panel-host'

/**
 * Resolves one native panel's selection. Contextual documents have immutable
 * tab identity in their outer URL; window panels retain an independent chat.
 * Selection changes remount the chat lifetime, while new runs preserve drafts.
 */
export function usePanelConversation() {
  const [tabId] = useState(() => panelTabIdFromUrl(window.location.href))
  const [view, setView] = useState<ConversationPanelView | null>(null)
  const windowId = useRef<number | undefined>(undefined)
  const revision = useRef(0)
  const pendingSelection = useRef<string | undefined>(undefined)
  const [searchParams, setSearchParams] = useSearchParams()
  const historyId = searchParams.get('conversationId')

  const select = useCallback(
    (conversationId: string) => {
      revision.current += 1
      pendingSelection.current = conversationId
      setView({ tabId: tabId ?? -1, conversationId, manual: true })
      const persist =
        tabId === undefined
          ? windowId.current === undefined
            ? Promise.resolve()
            : setWindowConversation(windowId.current, conversationId)
          : sendRuntimeMessage(RuntimeMessageType.selectPanelConversation, {
              tabId,
              conversationId,
            })
      void persist.catch((error) => sentry.captureException(error))
    },
    [tabId],
  )

  useEffect(() => {
    let cancelled = false
    const initialRevision = revision.current
    const unwatch =
      tabId === undefined
        ? undefined
        : conversationPanelViewsStorage.watch((views) => {
            const selected = views[String(tabId)]
            if (!cancelled && selected) {
              if (
                pendingSelection.current &&
                selected.conversationId !== pendingSelection.current
              )
                return
              pendingSelection.current = undefined
              revision.current += 1
              setView(selected)
            }
          })
    void (async () => {
      let selected: ConversationPanelView | undefined
      if (tabId !== undefined) {
        selected = (await conversationPanelViewsStorage.getValue())[
          String(tabId)
        ]
      } else {
        const current = await chrome.windows.getCurrent()
        windowId.current = current.id
        const conversationId =
          current.id === undefined
            ? null
            : await getWindowConversation(current.id)
        if (conversationId)
          selected = { tabId: -1, conversationId, manual: true }
      }
      if (cancelled || revision.current !== initialRevision) return
      if (selected) setView(selected)
      else select(crypto.randomUUID())
    })().catch((error) => {
      sentry.captureException(error)
      if (!cancelled && revision.current === initialRevision)
        select(crypto.randomUUID())
    })
    return () => {
      cancelled = true
      unwatch?.()
    }
  }, [tabId, select])

  useEffect(() => {
    if (!historyId) return
    select(historyId)
    setSearchParams({}, { replace: true })
  }, [historyId, select, setSearchParams])

  return { tabId, view, select }
}
