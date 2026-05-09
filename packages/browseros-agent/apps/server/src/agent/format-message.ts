import type { BrowserContext } from '@browseros/shared/schemas/browser-context'

export interface UserContextAttachment {
  kind: 'file' | 'memory'
  title: string
  source?: string
  content: string
}

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

/** Strip XML-like tags that match our prompt delimiters to prevent injection. */
function sanitizeForPrompt(s: string): string {
  return s.replace(
    /<\/?(?:selected_text|attached_context|USER_QUERY|page_context|AGENT_PROMPT|soul|memory_and_identity|security|workspace)[^>]*>/gi,
    '',
  )
}

function sanitizeAttribute(s: string): string {
  return sanitizeForPrompt(s).replace(/["<>]/g, "'")
}

function formatContextAttachments(
  attachments?: ReadonlyArray<UserContextAttachment>,
): string {
  if (!attachments?.length) return ''

  return attachments
    .filter((attachment) => attachment.content.trim())
    .map((attachment) => {
      const title = sanitizeAttribute(attachment.title)
      const source = attachment.source
        ? ` source="${sanitizeAttribute(attachment.source)}"`
        : ''
      const content = sanitizeForPrompt(attachment.content)
      return `<attached_context type="${attachment.kind}" title="${title}"${source}>\n${content}\n</attached_context>`
    })
    .join('\n\n')
}

export function formatUserMessage(
  message: string,
  browserContext?: BrowserContext,
  selectedText?: string,
  selectedTextSource?: { url: string; title: string },
  contextAttachments?: ReadonlyArray<UserContextAttachment>,
): string {
  const contextPrefix = formatBrowserContext(browserContext)

  let selectedTextBlock = ''
  if (selectedText) {
    const sanitizedText = sanitizeForPrompt(selectedText)
    const title = selectedTextSource?.title
      ? sanitizeForPrompt(selectedTextSource.title).replace(/"/g, "'")
      : ''
    const url = selectedTextSource?.url
      ? sanitizeForPrompt(selectedTextSource.url)
      : ''
    const source = title ? ` (from "${title}"${url ? ` — ${url}` : ''})` : ''
    selectedTextBlock = `<selected_text${source}>\n${sanitizedText}\n</selected_text>\n\n`
  }

  const contextAttachmentBlock = formatContextAttachments(contextAttachments)
  const contextAttachmentPrefix = contextAttachmentBlock
    ? `${contextAttachmentBlock}\n\n`
    : ''

  return `${contextPrefix}${selectedTextBlock}${contextAttachmentPrefix}<USER_QUERY>\n${message}\n</USER_QUERY>`
}
