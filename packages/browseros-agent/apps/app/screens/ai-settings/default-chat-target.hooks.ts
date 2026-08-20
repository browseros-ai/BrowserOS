import { useEffect, useState } from 'react'
import type { LlmProviderConfig } from '@/lib/llm-providers/types'
import { sentry } from '@/lib/sentry/sentry'
import {
  commitChatTargetSelection,
  loadSidepanelChatTargetSelection,
  type SidepanelChatTargetSelection,
  watchSidepanelChatTargetSelection,
} from '@/modules/chat/sidepanel-chat-targets'
import { resolveEffectiveDefaultTarget } from './default-chat-target.helpers'

export interface UseDefaultChatTargetInput {
  providers: LlmProviderConfig[]
  agents: ReadonlyArray<{ id: string }>
  defaultProviderId: string
  setDefaultProvider: (providerId: string) => Promise<void>
}

export interface DefaultChatTargetController {
  effectiveTarget: SidepanelChatTargetSelection
  selectProvider: (providerId: string) => void
  selectAgent: (agentId: string) => void
  selectTarget: (selection: SidepanelChatTargetSelection) => void
}

/**
 * Selection state for the AI-settings pane's unified default-target radio
 * group. Reads and writes the same persisted selection the sidepanel resolves
 * (`local:sidepanel-chat-target-selection`), so picking a row here changes
 * what new chats use everywhere. Selecting a provider also updates the
 * default-provider id, mirroring the sidepanel's select semantics.
 */
export function useDefaultChatTarget({
  providers,
  agents,
  defaultProviderId,
  setDefaultProvider,
}: UseDefaultChatTargetInput): DefaultChatTargetController {
  const [selection, setSelection] =
    useState<SidepanelChatTargetSelection | null>(null)

  useEffect(() => {
    let cancelled = false
    loadSidepanelChatTargetSelection()
      .then((stored) => {
        if (!cancelled) setSelection(stored)
      })
      .catch((error) => {
        sentry.captureException(error, {
          extra: { message: 'Failed to load default chat-target selection' },
        })
      })
    const unwatch = watchSidepanelChatTargetSelection((stored) => {
      setSelection(stored)
    })
    return () => {
      cancelled = true
      unwatch()
    }
  }, [])

  const selectTarget = (next: SidepanelChatTargetSelection) => {
    setSelection(next)
    commitChatTargetSelection(next, { setDefaultProvider }).catch((error) => {
      sentry.captureException(error, {
        extra: {
          message: 'Failed to change default chat target',
          targetId: next.id,
          targetKind: next.kind,
        },
      })
    })
  }

  const selectProvider = (providerId: string) => {
    selectTarget({ kind: 'llm', id: providerId })
  }

  const selectAgent = (agentId: string) => {
    selectTarget({ kind: 'acp', id: agentId })
  }

  const effectiveTarget = resolveEffectiveDefaultTarget({
    providers,
    agents,
    selection,
    defaultProviderId,
  })

  return { effectiveTarget, selectProvider, selectAgent, selectTarget }
}
