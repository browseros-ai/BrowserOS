import { defineExtensionMessaging } from '@webext-core/messaging'

export const RuntimeMessageType = {
  getTabId: 'runtime.getTabId',
  authSuccess: 'runtime.authSuccess',
  stopAgent: 'runtime.stopAgent',
  sidePanelScopeChanged: 'runtime.sidePanelScopeChanged',
} as const

export interface RuntimeTabIdResponse {
  tabId?: number
}

export interface RuntimeStopAgentData {
  conversationId: string
}

export interface RuntimeSidePanelScopeChangedData {
  perWindow: boolean
}

type RuntimeMessagesProtocol = {
  [RuntimeMessageType.getTabId](): RuntimeTabIdResponse
  [RuntimeMessageType.authSuccess](): void
  [RuntimeMessageType.stopAgent](data: RuntimeStopAgentData): void
  [RuntimeMessageType.sidePanelScopeChanged](
    data: RuntimeSidePanelScopeChangedData,
  ): void
}

const { sendMessage, onMessage } =
  defineExtensionMessaging<RuntimeMessagesProtocol>()

export { onMessage as onRuntimeMessage, sendMessage as sendRuntimeMessage }
