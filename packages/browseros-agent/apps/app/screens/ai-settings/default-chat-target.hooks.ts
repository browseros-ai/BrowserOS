import type { LlmProviderConfig } from '@/lib/llm-providers/types'
import { sentry } from '@/lib/sentry/sentry'
import type { ChatTargetRef } from '@/modules/chat/sidepanel-chat-targets'
import { resolveEffectiveDefaultTarget } from './default-chat-target.helpers'

export interface UseDefaultChatTargetInput {
  providers: LlmProviderConfig[]
  agents: ReadonlyArray<{ id: string }>
  /** The stored id verbatim, which names a row of either kind. */
  defaultTargetId: string | null
  setDefaultProvider: (providerId: string) => Promise<void>
}

export interface DefaultChatTargetController {
  /** Null when nothing is configured: the radio group then shows no selection. */
  effectiveTarget: ChatTargetRef | null
  selectProvider: (providerId: string) => void
  selectAgent: (agentId: string) => void
  selectTarget: (target: ChatTargetRef) => void
}

/**
 * Selection state for the AI-settings pane's unified default-target radio
 * group. It writes the same default the sidepanel resolves, so picking a row
 * here changes what new chats use everywhere.
 */
export function useDefaultChatTarget({
  providers,
  agents,
  defaultTargetId,
  setDefaultProvider,
}: UseDefaultChatTargetInput): DefaultChatTargetController {
  const selectTarget = (next: ChatTargetRef) => {
    setDefaultProvider(next.id).catch((error) => {
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
    defaultTargetId,
  })

  return { effectiveTarget, selectProvider, selectAgent, selectTarget }
}
