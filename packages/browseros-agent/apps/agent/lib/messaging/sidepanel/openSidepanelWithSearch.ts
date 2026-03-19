import { defineExtensionMessaging } from '@webext-core/messaging'
import type { SearchActionStorage } from '@/lib/search-actions/searchActionsStorage'

type OpenSidePanelWithSearchParams = {
  open(props: SearchActionStorage): void
}

/**
 * @public
 */
const { sendMessage, onMessage } =
  defineExtensionMessaging<OpenSidePanelWithSearchParams>()

export {
  onMessage as onOpenSidePanelWithSearch,
  sendMessage as openSidePanelWithSearch,
}
