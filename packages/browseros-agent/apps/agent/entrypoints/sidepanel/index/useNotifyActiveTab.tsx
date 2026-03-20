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

export const useNotifyActiveTab = ({
  messages,
  status,
  conversationId,
}: {
  messages: UIMessage[]
  status: ChatStatus
  conversationId: string
}) => {
  const activeTabIdsRef = useRef<Set<number>>(new Set())

  const lastMessage = messages?.[messages.length - 1]

  const latestTool =
    lastMessage?.parts?.findLast((part) => part?.type?.startsWith('tool-')) ??
    null

  const hasToolCalls = !!latestTool
  const toolTabId = extractTabId(latestTool as ToolUIPart | null)

  useEffect(() => {
    const isStreaming = status === 'streaming'
    const activeIds = activeTabIdsRef.current

    if (!isStreaming) {
      // Deactivate all tracked tabs when streaming stops
      if (activeIds.size > 0) {
        const deactivate = async () => {
          const alreadyShown = await firstRunConfettiShownStorage.getValue()
          let showConfetti = !alreadyShown

          for (const tabId of activeIds) {
            const deactivateMessage: GlowMessage = {
              conversationId,
              isActive: false,
              showConfetti,
            }
            chrome.tabs.sendMessage(tabId, deactivateMessage).catch(() => {})
            // Only show confetti on the first tab
            showConfetti = false
          }

          if (!alreadyShown) {
            await firstRunConfettiShownStorage.setValue(true)
          }
        }
        deactivate()
        activeIds.clear()
      }
      return
    }

    if (!hasToolCalls) return

    let cancelled = false

    const activate = async () => {
      let targetTabId = toolTabId ?? undefined

      if (!targetTabId) {
        // Fallback: use the last activated tab, or query the active tab
        const lastActive = [...activeIds].pop()
        if (lastActive) {
          targetTabId = lastActive
        } else {
          const tabs = await chrome.tabs.query({
            active: true,
            currentWindow: true,
          })
          targetTabId = tabs[0]?.id
        }
      }

      if (cancelled || !targetTabId) return

      // Send glow to the target tab (may already be active — idempotent)
      const activateMessage: GlowMessage = {
        conversationId,
        isActive: true,
      }
      chrome.tabs.sendMessage(targetTabId, activateMessage).catch(() => {})
      activeIds.add(targetTabId)
    }

    activate()

    return () => {
      cancelled = true
    }
  }, [conversationId, status, hasToolCalls, toolTabId])

  return
}
