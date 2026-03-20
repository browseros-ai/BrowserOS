import type { ChatStatus, ToolUIPart, UIMessage } from 'ai'
import { useEffect, useRef } from 'react'
import type { GlowMessage } from '@/entrypoints/glow.content/GlowMessage'
import { firstRunConfettiShownStorage } from '@/lib/onboarding/onboardingStorage'

function extractTabId(toolPart: ToolUIPart | null): number | undefined {
  if (!toolPart) return undefined

  // CDP tools: server includes tabId in tool output metadata
  const output = (
    toolPart as ToolUIPart & {
      output?: { metadata?: { tabId?: number } }
    }
  )?.output
  if (output?.metadata?.tabId) return output.metadata.tabId

  // Legacy controller tools: tabId in input
  const input = (toolPart as ToolUIPart & { input?: { tabId?: number } })?.input
  return input?.tabId
}

function sendGlow(tabId: number, message: GlowMessage): void {
  chrome.tabs.sendMessage(tabId, message).catch(() => {})
}

export const useNotifyActiveTab = ({
  messages,
  status,
  conversationId,
}: {
  messages: UIMessage[]
  status: ChatStatus
  conversationId: string
}) => {
  // Track the single tab currently glowing
  const activeTabIdRef = useRef<number | null>(null)
  // Track all tabs that have been glowed during this stream (for cleanup)
  const allGlowedTabsRef = useRef<Set<number>>(new Set())

  const lastMessage = messages?.[messages.length - 1]

  const latestTool =
    lastMessage?.parts?.findLast((part) => part?.type?.startsWith('tool-')) ??
    null

  const hasToolCalls = !!latestTool
  const toolTabId = extractTabId(latestTool as ToolUIPart | null)

  useEffect(() => {
    const isStreaming = status === 'streaming'

    if (!isStreaming) {
      // Deactivate ALL tabs that were glowed during this stream
      const allGlowed = allGlowedTabsRef.current
      if (allGlowed.size > 0) {
        const deactivate = async () => {
          // Capture tab IDs before any async work to avoid race with clear()
          const tabIds = Array.from(allGlowed)
          allGlowed.clear()

          const alreadyShown = await firstRunConfettiShownStorage.getValue()
          let showConfetti = !alreadyShown

          for (const tabId of tabIds) {
            sendGlow(tabId, {
              conversationId,
              isActive: false,
              showConfetti,
            })
            showConfetti = false
          }

          if (!alreadyShown) {
            await firstRunConfettiShownStorage.setValue(true)
          }
        }
        deactivate()
      }
      activeTabIdRef.current = null
      return
    }

    if (!hasToolCalls) return

    let cancelled = false

    const activate = async () => {
      let targetTabId = toolTabId ?? undefined

      if (!targetTabId) {
        // Fallback: use the currently active tab, or query browser
        if (activeTabIdRef.current) {
          targetTabId = activeTabIdRef.current
        } else {
          const tabs = await chrome.tabs.query({
            active: true,
            currentWindow: true,
          })
          targetTabId = tabs[0]?.id
        }
      }

      if (cancelled || !targetTabId) return

      const previousTabId = activeTabIdRef.current

      // If the agent moved to a different tab, deactivate the previous one
      if (previousTabId && previousTabId !== targetTabId) {
        sendGlow(previousTabId, {
          conversationId,
          isActive: false,
        })
      }

      // Activate glow on the target tab
      sendGlow(targetTabId, {
        conversationId,
        isActive: true,
      })

      activeTabIdRef.current = targetTabId
      allGlowedTabsRef.current.add(targetTabId)
    }

    activate()

    return () => {
      cancelled = true
    }
  }, [conversationId, status, hasToolCalls, toolTabId])

  return
}
