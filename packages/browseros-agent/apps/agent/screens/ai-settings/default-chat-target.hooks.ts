import { useEffect, useState } from 'react'
import type { LlmProviderConfig } from '@/lib/llm-providers/types'
import { sentry } from '@/lib/sentry/sentry'
import {
  loadSidepanelChatTargetSelection,
  type SidepanelChatTargetSelection,
  saveSidepanelChatTargetSelection,
  watchSidepanelChatTargetSelection,
} from '@/modules/chat/sidepanel-chat-targets'
import { resolveEffectiveDefaultTarget } from './default-chat-target.helpers'

interface UseDefaultChatTargetInput {
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

  const persistSelection = (next: SidepanelChatTargetSelection) => {
    setSelection(next)
    saveSidepanelChatTargetSelection(next).catch((error) => {
      sentry.captureException(error, {
        extra: {
          message: 'Failed to persist default chat-target selection',
          targetId: next.id,
          targetKind: next.kind,
        },
      })
    })
  }

  const selectProvider = (providerId: string) => {
    setDefaultProvider(providerId).catch((error) => {
      sentry.captureException(error, {
        extra: {
          message: 'Failed to persist default provider id',
          providerId,
        },
      })
    })
    persistSelection({ kind: 'llm', id: providerId })
  }

  const selectAgent = (agentId: string) => {
    persistSelection({ kind: 'acp', id: agentId })
  }

  const selectTarget = (next: SidepanelChatTargetSelection) => {
    if (next.kind === 'llm') {
      selectProvider(next.id)
    } else {
      selectAgent(next.id)
    }
  }

  const effectiveTarget = resolveEffectiveDefaultTarget({
    providers,
    agents,
    selection,
    defaultProviderId,
  })

  return { effectiveTarget, selectProvider, selectAgent, selectTarget }
}
