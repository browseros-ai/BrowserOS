import type { Provider } from '../../components/chat/chatComponentTypes'
import type { LlmProviderConfig } from '../../lib/llm-providers/types'
import { buildChatRequestBody } from '../../lib/messaging/server/buildChatRequestBody'
import {
  type SidepanelChatTarget,
  toLlmProviderConfig,
} from './sidepanel-chat-targets'

export type LlmChatRequestBodyInput = Parameters<typeof buildChatRequestBody>[0]

export type CommonSidepanelRequestInput = Omit<
  LlmChatRequestBodyInput,
  'provider' | 'message' | 'isScheduledTask'
>

export interface BuildSidepanelPreparedSendMessagesRequestInput
  extends CommonSidepanelRequestInput {
  agentServerUrl: string
  target: SidepanelChatTarget | undefined
  fallbackProvider: LlmProviderConfig
  agentSessionId?: string
  message?: string
}

export function buildSidepanelPreparedSendMessagesRequest({
  agentServerUrl,
  target,
  fallbackProvider,
  agentSessionId,
  message,
  ...common
}: BuildSidepanelPreparedSendMessagesRequestInput) {
  if (target?.kind === 'acp') {
    return {
      api: `${agentServerUrl}/agents/${encodeURIComponent(target.agentId)}/sidepanel/chat`,
      body: {
        conversationId: common.conversationId,
        agentSessionId: agentSessionId ?? common.conversationId,
        message: message ?? '',
        browserContext: common.browserContext,
        userSystemPrompt: common.userSystemPrompt,
        userWorkingDir: common.userWorkingDir,
        selectedText: common.selectedText,
        selectedTextSource: common.selectedTextSource,
      },
    }
  }

  const provider = toLlmProviderConfig(target) ?? fallbackProvider
  return {
    api: `${agentServerUrl}/chat`,
    body: buildChatRequestBody({
      ...common,
      provider,
      message,
    }),
  }
}

export function toProviderOption(target: SidepanelChatTarget): Provider {
  return {
    id: target.id,
    name: target.name,
    type: target.type,
    kind: target.kind,
    agentId: target.kind === 'acp' ? target.agentId : undefined,
    adapterName: target.kind === 'acp' ? target.adapterName : undefined,
    modelLabel: target.kind === 'acp' ? target.modelLabel : undefined,
    modelControl: target.kind === 'acp' ? target.modelControl : undefined,
  }
}
