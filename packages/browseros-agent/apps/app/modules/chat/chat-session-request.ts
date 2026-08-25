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
  message?: string
  attachments?: Array<{ mediaType: string; data: string }>
}

export interface PrepareSidepanelSendMessagesRequestInput
  extends Omit<
    BuildSidepanelPreparedSendMessagesRequestInput,
    'agentServerUrl'
  > {
  resolveAgentServerUrl: () => Promise<string>
}

export async function prepareSidepanelSendMessagesRequest({
  resolveAgentServerUrl,
  ...input
}: PrepareSidepanelSendMessagesRequestInput) {
  const agentServerUrl = await resolveAgentServerUrl()
  return buildSidepanelPreparedSendMessagesRequest({
    agentServerUrl,
    ...input,
  })
}

export function buildSidepanelPreparedSendMessagesRequest({
  agentServerUrl,
  target,
  fallbackProvider,
  message,
  ...common
}: BuildSidepanelPreparedSendMessagesRequestInput) {
  if (target?.kind === 'acp') {
    return {
      api: `${agentServerUrl}/chat`,
      body: {
        target: { type: target.agentType, agentId: target.agentId },
        conversationId: common.conversationId,
        message: message ?? '',
        mode: common.mode,
        browserContext: common.browserContext,
        userSystemPrompt: common.userSystemPrompt,
        userWorkingDir: common.userWorkingDir,
        supportsImages: common.supportsImages,
        previousConversation: common.previousConversation,
        declinedApps: common.declinedApps,
        selectedText: common.selectedText,
        selectedTextSource: common.selectedTextSource,
        attachments: common.attachments?.length
          ? common.attachments
          : undefined,
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
    brandKey: target.kind === 'acp' ? target.brandKey : undefined,
    modelLabel: target.kind === 'acp' ? target.modelLabel : undefined,
  }
}
