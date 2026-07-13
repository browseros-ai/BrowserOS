export type ChatMode = 'chat' | 'agent'

export interface Suggestion {
  display: string
  prompt: string
  icon: string
}

export const CHAT_SUGGESTIONS: Suggestion[] = [
  {
    display: 'Summarize this page',
    prompt: 'Read the current tab and summarize it in bullet points',
    icon: '✨',
  },
  {
    display: 'What topics does this page talk about?',
    prompt:
      'Read the current tab and briefly describe what it is about in 1-2 lines',
    icon: '🔍',
  },
  {
    display: 'Extract comments from this page',
    prompt: 'Read the current tab and extract comments as bullet points',
    icon: '💬',
  },
]

export const AGENT_SUGGESTIONS: Suggestion[] = [
  {
    display: 'Summarize this page and save it',
    prompt:
      'Summarize the current tab, preserve the source URL, and save the result to my Request Browser workspace',
    icon: '❤️',
  },
  {
    display: 'Extract visible data into a table',
    prompt:
      'Extract the visible list or table from the current tab into a structured Request Browser workspace database, including source links',
    icon: '⭐',
  },
  {
    display: 'Research prices on this page',
    prompt:
      'Find product or service prices on the current tab, flag missing or uncertain values, and prepare them for review before saving',
    icon: '🛒',
  },
]
