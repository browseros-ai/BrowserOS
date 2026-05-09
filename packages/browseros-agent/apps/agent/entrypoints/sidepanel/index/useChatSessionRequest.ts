import type { Provider } from '../../../components/chat/chatComponentTypes'
import type { LlmProviderConfig } from '../../../lib/llm-providers/types'
import {
  type ApprovalResponseData,
  buildChatRequestBody,
} from '../../../lib/messaging/server/buildChatRequestBody'
import {
  type SidepanelChatTarget,
  toLlmProviderConfig,
} from './sidepanel-chat-targets'

type LlmChatRequestBodyInput = Parameters<typeof buildChatRequestBody>[0]

type CommonSidepanelRequestInput = Omit<
  LlmChatRequestBodyInput,
  'provider' | 'message' | 'toolApprovalResponses' | 'isScheduledTask'
>

interface BuildSidepanelPreparedSendMessagesRequestInput
  extends CommonSidepanelRequestInput {
  agentServerUrl: string | undefined
  target: SidepanelChatTarget | undefined
  fallbackProvider: LlmProviderConfig
  message?: string
  approvalResponses?: ApprovalResponseData[] | null
}

export function buildSidepanelPreparedSendMessagesRequest({
  agentServerUrl,
  target,
  fallbackProvider,
  message,
  approvalResponses,
  ...common
}: BuildSidepanelPreparedSendMessagesRequestInput) {
  if (target?.kind === 'acp') {
    const body = {
      conversationId: common.conversationId,
      message: message ?? '',
      browserContext: common.browserContext,
      userSystemPrompt: common.userSystemPrompt,
      userWorkingDir: common.userWorkingDir,
      selectedText: common.selectedText,
      selectedTextSource: common.selectedTextSource,
      ...(common.contextAttachments?.length
        ? { contextAttachments: common.contextAttachments }
        : {}),
    }

    return {
      api: `${agentServerUrl}/agents/${encodeURIComponent(target.agentId)}/sidepanel/chat`,
      body,
    }
  }

  const provider = toLlmProviderConfig(target) ?? fallbackProvider
  return {
    api: `${agentServerUrl}/chat`,
    body: buildChatRequestBody({
      ...common,
      provider,
      message,
      toolApprovalResponses: approvalResponses ?? undefined,
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
