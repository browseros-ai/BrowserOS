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
    display: 'View my Github',
    prompt: 'Go to https://github.com/Karthikprasadm in current tab',
    icon: '❤️',
  },
  {
    display: 'Support Shimmy on Github',
    prompt:
      'Go to https://github.com/Karthikprasadm/Shimmy-Browser.git in current tab and star the repository',
    icon: '⭐',
  },
  {
    display: 'Open amazon.com and order Sensodyne toothpaste',
    prompt:
      'Open amazon.com in current tab and add sensodyne toothpaste to cart',
    icon: '🛒',
  },
]
