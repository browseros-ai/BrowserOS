/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Maps raw tool names + arguments to human-readable activity labels for
 * the chat UI activity view. The MCP ToolRegistry is the source of truth
 * for tool *existence*; this file is the editorial layer that turns
 * snake_case identifiers into agent-speak verbs.
 */

const VERB_OVERRIDES: Record<string, string> = {
  // Navigation
  navigate_page: 'Navigated to',
  new_page: 'Opened tab',
  new_hidden_page: 'Opened tab',
  show_page: 'Showed tab',
  close_page: 'Closed tab',
  list_pages: 'Listed open tabs',
  get_active_page: 'Got active tab',
  move_page: 'Moved tab',
  group_tabs: 'Grouped tabs',

  // Page reading
  take_snapshot: 'Captured page snapshot',
  take_enhanced_snapshot: 'Captured detailed snapshot',
  get_page_content: 'Read page content',
  get_page_links: 'Extracted page links',
  get_dom: 'Read page DOM',
  search_dom: 'Searched page DOM',
  take_screenshot: 'Took screenshot',

  // Input
  click: 'Clicked',
  click_at: 'Clicked at coordinates',
  hover: 'Hovered',
  hover_at: 'Hovered at coordinates',
  type_at: 'Typed at coordinates',
  drag_at: 'Dragged',
  focus: 'Focused element',
  fill: 'Filled field',
  clear: 'Cleared field',
  check: 'Checked box',
  uncheck: 'Unchecked box',
  press_key: 'Pressed key',
  upload_file: 'Uploaded file',

  // Scripts
  evaluate_script: 'Ran script',

  // History / bookmarks
  search_history: 'Searched history',
  get_recent_history: 'Read recent history',
  delete_history_url: 'Deleted history entry',
  delete_history_range: 'Deleted history range',
  get_bookmarks: 'Listed bookmarks',
  create_bookmark: 'Created bookmark',
  remove_bookmark: 'Removed bookmark',
  update_bookmark: 'Updated bookmark',
  move_bookmark: 'Moved bookmark',
  search_bookmarks: 'Searched bookmarks',

  // Filesystem (sandboxed)
  read_file: 'Read file',
  write_file: 'Wrote file',
  find_files: 'Searched files',

  // Web
  web_search: 'Searched the web',
  web_fetch: 'Fetched URL',

  // Klavis / external apps (Strata)
  connector_mcp_servers: 'Listed connected apps',
  discover_server_categories_or_actions: 'Browsed available actions',
  get_category_actions: 'Listed actions',
  get_action_details: 'Looked up action',
  execute_action: 'Ran external action',
  search_documentation: 'Searched docs',
  handle_auth_failure: 'Handled auth issue',

  // Suggestions
  suggest_schedule: 'Suggested schedule',
  suggest_app_connection: 'Suggested app connect',

  // BrowserOS info
  browseros_info: 'Read BrowserOS info',

  // Windows
  list_windows: 'Listed windows',
  focus_window: 'Focused window',
  close_window: 'Closed window',
  create_window: 'Created window',
}

// ──────────────────────────────────────────────────────────────────────
// Helpers
// ──────────────────────────────────────────────────────────────────────

function asString(value: unknown): string | undefined {
  return typeof value === 'string' && value.length > 0 ? value : undefined
}

function stringField(
  input: Record<string, unknown>,
  ...keys: string[]
): string | undefined {
  for (const k of keys) {
    const v = asString(input[k])
    if (v) return v
  }
  return undefined
}

function truncate(text: string | undefined, max: number): string | undefined {
  if (!text) return undefined
  return text.length > max ? `${text.slice(0, max - 1)}…` : text
}

function quote(value: string | undefined): string | undefined {
  if (!value) return undefined
  return `"${truncate(value, 60)}"`
}

function basename(path: string | undefined): string | undefined {
  if (!path) return undefined
  const parts = path.split(/[/\\]/).filter(Boolean)
  return parts[parts.length - 1] ?? path
}

function formatUrl(value: unknown): string | undefined {
  const url = asString(value)
  if (!url) return undefined
  try {
    const parsed = new URL(url)
    const host = parsed.host
    const path = parsed.pathname === '/' ? '' : parsed.pathname
    const display = path && path.length > 0 ? `${host}${path}` : host
    return truncate(display, 60)
  } catch {
    return truncate(url, 60)
  }
}

function coords(x: unknown, y: unknown): string | undefined {
  if (typeof x === 'number' && typeof y === 'number') {
    return `${Math.round(x)}, ${Math.round(y)}`
  }
  return undefined
}

// ──────────────────────────────────────────────────────────────────────
// Subject extractors
// ──────────────────────────────────────────────────────────────────────

type SubjectExtractor = (input: Record<string, unknown>) => string | undefined

const SUBJECT_EXTRACTORS: Record<string, SubjectExtractor> = {
  // URL-bearing tools
  new_page: (i) => formatUrl(i.url),
  new_hidden_page: (i) => formatUrl(i.url),
  navigate_page: (i) => {
    const action = asString(i.action)
    if (action === 'back') return 'back'
    if (action === 'forward') return 'forward'
    if (action === 'reload') return 'reload'
    return formatUrl(i.url)
  },
  web_fetch: (i) => formatUrl(i.url),

  // Search queries
  web_search: (i) => quote(stringField(i, 'query', 'q')),
  search_history: (i) => quote(stringField(i, 'query', 'text')),
  search_bookmarks: (i) => quote(stringField(i, 'query', 'text')),
  search_dom: (i) => quote(stringField(i, 'query', 'selector')),
  search_documentation: (i) => quote(stringField(i, 'query', 'q')),
  find_files: (i) => quote(stringField(i, 'pattern', 'query')),

  // Element interactions
  click: (i) => stringField(i, 'element'),
  hover: (i) => stringField(i, 'element'),
  focus: (i) => stringField(i, 'element'),
  clear: (i) => stringField(i, 'element'),
  check: (i) => stringField(i, 'element'),
  uncheck: (i) => stringField(i, 'element'),
  fill: (i) => {
    const target = stringField(i, 'element')
    const text = stringField(i, 'text')
    if (target && text) return `${target}: ${truncate(text, 40)}`
    return target ?? truncate(text, 40)
  },
  press_key: (i) => stringField(i, 'key'),

  // Coordinate-based input
  click_at: (i) => coords(i.x, i.y),
  hover_at: (i) => coords(i.x, i.y),
  type_at: (i) => {
    const at = coords(i.x, i.y)
    const text = stringField(i, 'text')
    if (at && text) return `${at}: ${truncate(text, 40)}`
    return at ?? truncate(text, 40)
  },
  drag_at: (i) => {
    const from = coords(i.fromX, i.fromY)
    const to = coords(i.toX, i.toY)
    if (from && to) return `${from} → ${to}`
    return from ?? to
  },

  // Tab management
  show_page: (i) => {
    const page = i.page
    return typeof page === 'number' ? `tab ${page}` : asString(page)
  },
  close_page: (i) => {
    const page = i.page
    return typeof page === 'number' ? `tab ${page}` : asString(page)
  },
  move_page: (i) => {
    const page = i.page
    return typeof page === 'number' ? `tab ${page}` : asString(page)
  },

  // Page reads (take_snapshot, take_enhanced_snapshot, get_page_content,
  // get_page_links, get_dom, take_screenshot) intentionally omit a
  // subject — the only argument is a numeric page ID that's internal
  // to the agent and meaningless to the user ("tab 4" tells them nothing).
  // The verb alone communicates what happened.

  // External actions via Strata
  execute_action: (i) => {
    const server = stringField(i, 'server_name')
    const action = stringField(i, 'action_name')
    if (server && action) return `${server} · ${action}`
    return action ?? server
  },
  get_category_actions: (i) => stringField(i, 'category_name', 'server_name'),
  get_action_details: (i) => stringField(i, 'action_name'),
  discover_server_categories_or_actions: (i) =>
    stringField(i, 'server_name', 'category_name'),
  connector_mcp_servers: (i) => stringField(i, 'server_name'),

  // Filesystem
  read_file: (i) => basename(stringField(i, 'path')),
  write_file: (i) => basename(stringField(i, 'path')),

  // Bookmarks
  create_bookmark: (i) => stringField(i, 'title') ?? formatUrl(i.url),
  remove_bookmark: (i) => stringField(i, 'id', 'title'),
  update_bookmark: (i) => stringField(i, 'id', 'title'),
  move_bookmark: (i) => stringField(i, 'id', 'title'),

  // History
  delete_history_url: (i) => formatUrl(i.url),
}

// ──────────────────────────────────────────────────────────────────────
// Public API
// ──────────────────────────────────────────────────────────────────────

export interface ToolLabelResult {
  label: string
  subject?: string
}

/**
 * Strip MCP namespace prefixes (e.g. "browseros__", "mcp_") to find the
 * canonical tool name used in the override maps.
 */
function canonicalName(rawName: string): string {
  return rawName.replace(/^browseros__/, '').replace(/^mcp_/, '')
}

/**
 * Convert a snake_case tool name into Sentence-case English as a fallback
 * when no curated override exists.
 */
function humanizeToolName(rawName: string): string {
  const stripped = canonicalName(rawName)
  const words = stripped.split(/[_-]/).filter((w) => w.length > 0)
  const first = words[0]
  if (!first) return rawName
  return [
    first.charAt(0).toUpperCase() + first.slice(1),
    ...words.slice(1),
  ].join(' ')
}

/**
 * Build a human-readable label and subject string for a tool call,
 * suitable for rendering in the chat activity view.
 */
export function buildToolLabel(
  rawName: string,
  input?: Record<string, unknown>,
): ToolLabelResult {
  const canonical = canonicalName(rawName)
  const label =
    VERB_OVERRIDES[canonical] ??
    VERB_OVERRIDES[rawName] ??
    humanizeToolName(rawName)

  const extractor = Object.hasOwn(SUBJECT_EXTRACTORS, canonical)
    ? SUBJECT_EXTRACTORS[canonical]
    : Object.hasOwn(SUBJECT_EXTRACTORS, rawName)
      ? SUBJECT_EXTRACTORS[rawName]
      : undefined
  const subject = extractor && input ? extractor(input) : undefined

  return { label, subject }
}
