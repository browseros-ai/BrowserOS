import { z } from 'zod'
import type { ChatMode } from './chat-types'

/** Fixed shortcut slots shared by new-tab and side-panel conversations. */
export const STARTER_PROMPT_SLOTS = [0, 1, 2] as const
export const STARTER_PROMPT_LABEL_LIMIT = 80
export const STARTER_PROMPT_TEXT_LIMIT = 4000

export const starterPromptsSchema = z
  .array(
    z.object({
      display: z
        .string()
        .trim()
        .min(1, 'Enter a button label.')
        .max(STARTER_PROMPT_LABEL_LIMIT),
      prompt: z
        .string()
        .trim()
        .min(1, 'Enter a prompt.')
        .max(STARTER_PROMPT_TEXT_LIMIT),
    }),
  )
  .length(STARTER_PROMPT_SLOTS.length)

/** A compact button label and the complete instruction sent when it is run. */
export type StarterPrompt = z.infer<typeof starterPromptsSchema>[number]

const defaults: Record<ChatMode, StarterPrompt[]> = {
  chat: [
    {
      display: 'Summarize this page',
      prompt: 'Read the current tab and summarize it in bullet points',
    },
    {
      display: 'What topics does this page talk about?',
      prompt:
        'Read the current tab and briefly describe what it is about in 1-2 lines',
    },
    {
      display: 'Extract comments from this page',
      prompt: 'Read the current tab and extract comments as bullet points',
    },
  ],
  agent: [
    {
      display: 'Read about our vision and upvote',
      prompt:
        'Go to https://dub.sh/browseros-launch in current tab. Find and click the upvote button',
    },
    {
      display: 'Support BrowserOS on Github',
      prompt:
        'Go to http://git.new/browseros in current tab and star the repository',
    },
    {
      display: 'Open amazon.com and order Sensodyne toothpaste',
      prompt:
        'Open amazon.com in current tab and add sensodyne toothpaste to cart',
    },
  ],
}

export function getDefaultStarterPrompts(mode: ChatMode): StarterPrompt[] {
  return defaults[mode].map((prompt) => ({ ...prompt }))
}

/** Storage is an untrusted boundary: invalid preferences never break new chat. */
export function resolveStarterPrompts(
  mode: ChatMode,
  value: unknown,
): StarterPrompt[] {
  const result = starterPromptsSchema.safeParse(value)
  return result.success ? result.data : getDefaultStarterPrompts(mode)
}
