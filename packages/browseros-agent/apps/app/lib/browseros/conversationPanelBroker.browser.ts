import { firstRunConfettiShownStorage } from '@/lib/onboarding/onboardingStorage'
import { ConversationPanelBroker } from './conversationPanelBroker'
import { conversationPanelViewsStorage } from './conversationPanelStorage'
import { getAgentServerUrl } from './helpers'
import { openSidePanel } from './toggleSidePanel'

/** Binds the broker's narrow effects to extension APIs in the background realm. */
export function createConversationPanelBroker(): ConversationPanelBroker {
  return new ConversationPanelBroker({
    resolveServerUrl: getAgentServerUrl,
    fetch: (input, init) => fetch(input, init),
    getTab: (tabId) => chrome.tabs.get(tabId),
    openPanel: async (target) => {
      await openSidePanel(target)
    },
    readViews: () => conversationPanelViewsStorage.getValue(),
    writeViews: (views) => conversationPanelViewsStorage.setValue(views),
    sendGlow: (tabId, message) => {
      chrome.tabs.sendMessage(tabId, message).catch(() => undefined)
    },
    hasShownConfetti: () => firstRunConfettiShownStorage.getValue(),
    markConfettiShown: () => firstRunConfettiShownStorage.setValue(true),
    reportError: (error, context) => {
      // biome-ignore lint/suspicious/noConsole: MV3 background failures otherwise have no durable diagnostic surface.
      console.warn('[conversation-panel-broker]', context, error)
    },
  })
}
