import type { BrowserToolEffect } from '../browser-tool-dispatch'

/** Associates only pages this call actually used; `tabs list` yields no facts. */
export const applyConversationTabs: BrowserToolEffect = ({ call, result }) => {
  if (result.isError || !call.run || call.trace.touched.size === 0) return
  const tabIds = [...call.trace.touched]
    .map((pageId) => call.context.session.pages.getTabId(pageId))
    .filter((tabId): tabId is number => tabId !== undefined)
  call.run.associateTabs(tabIds)
}
