export const extractPath = (args: Record<string, unknown>): string | null => {
  for (const key of ['path', 'file_path', 'filePath', 'file']) {
    if (typeof args[key] === 'string') return args[key] as string
  }
  return null
}

export const extractUrl = (args: Record<string, unknown>): string | null => {
  if (typeof args.url === 'string' && args.url.length > 0) return args.url
  return null
}

export const extractPageId = (args: Record<string, unknown>): number | null => {
  if (typeof args.page === 'number') return args.page
  return null
}

const BROWSER_TOOLS = new Set([
  'click',
  'click_at',
  'fill',
  'type_at',
  'hover',
  'hover_at',
  'scroll',
  'drag',
  'drag_at',
  'focus',
  'clear',
  'press_key',
  'handle_dialog',
  'select_option',
  'upload_file',
  'take_snapshot',
  'take_enhanced_snapshot',
  'take_screenshot',
  'get_page_content',
  'get_page_links',
  'get_dom',
  'search_dom',
  'evaluate_script',
  'get_console_logs',
  'navigate_page',
  'new_page',
  'new_hidden_page',
  'get_active_page',
  'list_pages',
  'close_page',
  'move_page',
  'show_page',
  'save_pdf',
  'download_file',
])

export const isNavigationTool = (name: string): boolean =>
  name === 'navigate_page' || name === 'new_page' || name === 'new_hidden_page'

export const isBrowserTool = (name: string): boolean => BROWSER_TOOLS.has(name)

export const summarizeToolArgs = (args: Record<string, unknown>): string => {
  const path = extractPath(args)
  if (path) return `path=${path}`
  if (typeof args.command === 'string') return `command=${args.command}`
  if (typeof args.query === 'string') return `query=${args.query}`
  return Object.keys(args).join(', ')
}
