import type { BrowserContext } from '@browseros/shared/schemas/browser-context'

export function formatBrowserContext(browserContext?: BrowserContext): string {
  if (!browserContext?.activeTab && !browserContext?.selectedTabs?.length) {
    return ''
  }

  const formatTab = (tab: {
    id: number
    url?: string
    title?: string
    pageId?: number
  }) => {
    let line = `Tab ${tab.id}`
    if (tab.pageId !== undefined) line += ` (Page ID: ${tab.pageId})`
    if (tab.title) line += ` - "${tab.title}"`
    if (tab.url) line += ` (${tab.url})`
    return line
  }

  const lines: string[] = ['## Browser Context']

  if (browserContext.windowId !== undefined) {
    lines.push(`**Window ID:** ${browserContext.windowId}`)
  }

  if (browserContext.activeTab) {
    lines.push(`**Active Tab:** ${formatTab(browserContext.activeTab)}`)
  }

  if (browserContext.selectedTabs?.length) {
    lines.push(`**Selected Tabs (${browserContext.selectedTabs.length}):**`)
    browserContext.selectedTabs.forEach((tab, i) => {
      lines.push(`  ${i + 1}. ${formatTab(tab)}`)
    })
  }

  return `${lines.join('\n')}\n\n---\n\n`
}

export function formatUserMessage(
  message: string,
  browserContext?: BrowserContext,
  selectedText?: string,
  selectedTextSource?: { url: string; title: string },
): string {
  const contextPrefix = formatBrowserContext(browserContext)

  let selectedTextBlock = ''
  if (selectedText) {
    const source = selectedTextSource
      ? ` (from "${selectedTextSource.title}")`
      : ''
    selectedTextBlock = `<selected_text${source}>\n${selectedText}\n</selected_text>\n\n`
  }

  return `${contextPrefix}${selectedTextBlock}<USER_QUERY>\n${message}\n</USER_QUERY>`
}
