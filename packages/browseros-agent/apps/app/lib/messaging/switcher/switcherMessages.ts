import { defineExtensionMessaging } from '@webext-core/messaging'

interface SwitcherGetTabsResponse {
  tabs: chrome.tabs.Tab[]
  windows: chrome.windows.Window[]
  thumbnails: Record<number, string>
}

interface SwitcherSwitchData {
  tabId: number
  windowId: number
}

type SwitcherMessagesProtocol = {
  'tab-switcher:get-tabs'(): SwitcherGetTabsResponse
  'tab-switcher:switch'(data: SwitcherSwitchData): void
}

const { sendMessage, onMessage } =
  defineExtensionMessaging<SwitcherMessagesProtocol>()

export { onMessage as onSwitcherMessage, sendMessage as sendSwitcherMessage }
