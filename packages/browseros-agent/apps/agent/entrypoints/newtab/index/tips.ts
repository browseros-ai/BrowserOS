export interface Tip {
  id: string
  text: string
  shortcut?: string
}

export const TIP_SHOW_PROBABILITY = 0.5

const TIP_DISMISSED_KEY = 'tip-dismissed-session'

export const TIPS: Tip[] = [
  {
    id: 'chat-any-page',
    text: 'Press Ctrl+Shift+K (Cmd+Shift+K on Mac) to open the AI Chat panel on any webpage.',
    shortcut: '⌃⇧K',
  },
  {
    id: 'switch-models',
    text: 'Press Ctrl+Shift+L (Cmd+Shift+L on Mac) to cycle between AI providers without closing the chat panel.',
    shortcut: '⌃⇧L',
  },
  {
    id: 'screenshot-chat',
    text: 'Use the Image button in Chat to capture the visible page and ask visual questions about it.',
  },
  {
    id: 'copy-page-content',
    text: 'Click the Copy button in Chat to grab all webpage text and paste it into your prompt.',
  },
  {
    id: 'cowork-mode',
    text: 'Enable Cowork and select a folder to let the agent browse the web AND create files in a single task.',
  },
  {
    id: 'scheduled-tasks',
    text: 'Set up Scheduled Tasks to run the agent on a timer — results appear right here on your New Tab.',
  },
  {
    id: 'background-tasks',
    text: 'Scheduled tasks run in a separate window so they never interrupt your browsing.',
  },
  {
    id: 'claude-code-mcp',
    text: 'Connect BrowserOS to Claude Code to control tabs, clicks, and pages from your terminal.',
  },
  {
    id: 'mcp-servers',
    text: 'Add MCP servers for Google Calendar, Gmail, Notion, and more to power multi-service automations.',
  },
  {
    id: 'skills',
    text: 'Create a Skill if you want the agent to follow the same instructions every time for a task.',
  },
  {
    id: 'smart-nudges',
    text: 'If BrowserOS offers to connect an app, saying yes lets it use that app directly next time.',
  },
  {
    id: 'soul-md',
    text: "Tell the assistant things like 'be more direct' or 'always ask first,' and it updates your SOUL.md.",
  },
  {
    id: 'sync-to-cloud',
    text: 'Sign in to sync your chats, settings, and scheduled tasks across devices. API keys stay on each device.',
  },
  {
    id: 'import-chrome',
    text: 'Go to chrome://settings/importData to import bookmarks, passwords, and history from Chrome in one click.',
  },
  {
    id: 'ad-blocking',
    text: 'BrowserOS supports uBlock Origin for ad blocking — install it from the Chrome Web Store or GitHub.',
  },
  {
    id: 'at-mention-tabs',
    text: 'Type @ in the search bar to mention and attach open tabs as context for your AI queries.',
  },
  {
    id: 'mode-selection',
    text: 'Use Chat mode for read-only operations like questions and summaries, and Agent mode for multi-step browser tasks.',
  },
  {
    id: 'vertical-tabs',
    text: 'Turn on Vertical Tabs in Settings > Customization to move your tabs into a readable list on the left.',
  },
  {
    id: 'memory',
    text: "Say 'remember this' to save something important, or ask 'what do you remember about X?' later.",
  },
]

export const shouldShowTip = (): boolean => {
  const dismissed = sessionStorage.getItem(TIP_DISMISSED_KEY)
  if (dismissed) return false
  return Math.random() < TIP_SHOW_PROBABILITY
}

export const dismissTip = () => {
  sessionStorage.setItem(TIP_DISMISSED_KEY, Date.now().toString())
}

export const getRandomTip = (): Tip | null => {
  if (TIPS.length === 0) return null
  return TIPS[Math.floor(Math.random() * TIPS.length)]
}
