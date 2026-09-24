import { defineExtensionMessaging } from '@webext-core/messaging'

export const RuntimeMessageType = {
  getTabId: 'runtime.getTabId',
  stopAgent: 'runtime.stopAgent',
} as const

export interface RuntimeTabIdResponse {
  tabId?: number
}

export interface RuntimeStopAgentData {
  conversationId: string
}

type RuntimeMessagesProtocol = {
  [RuntimeMessageType.getTabId](): RuntimeTabIdResponse
  [RuntimeMessageType.stopAgent](data: RuntimeStopAgentData): void
}

const { sendMessage, onMessage } =
  defineExtensionMessaging<RuntimeMessagesProtocol>()

export { onMessage as onRuntimeMessage, sendMessage as sendRuntimeMessage }
