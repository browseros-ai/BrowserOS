import type { BrowserToolEffect } from '../browser-tool-dispatch'

/** Creation fallback for adapters that report page facts only after execution.
 * Merely reading or navigating a user tab must not replace its conversation. */
export const applyConversationTabs: BrowserToolEffect = ({ call, result }) => {
  if (result.isError || !call.run || call.trace.created.size === 0) return
  const tabIds = [...call.trace.created]
    .map((pageId) => call.context.session.pages.getTabId(pageId))
    .filter((tabId): tabId is number => tabId !== undefined)
  call.run.associateTabs(tabIds)
}
