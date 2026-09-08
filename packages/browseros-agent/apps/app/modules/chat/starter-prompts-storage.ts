import { storage } from '@wxt-dev/storage'
import type { ChatMode } from './chat-types'
import {
  getDefaultStarterPrompts,
  resolveStarterPrompts,
  type StarterPrompt,
  starterPromptsSchema,
} from './starter-prompts'

// Separate keys prevent saving Agent shortcuts from overwriting Chat shortcuts
// in another open extension surface. Each mode's three slots commit together.
const stores = {
  chat: storage.defineItem<StarterPrompt[]>('local:chatStarterPrompts', {
    fallback: getDefaultStarterPrompts('chat'),
  }),
  agent: storage.defineItem<StarterPrompt[]>('local:agentStarterPrompts', {
    fallback: getDefaultStarterPrompts('agent'),
  }),
}

/**
 * Observes preferences across extension surfaces, including the initial load.
 * The returned cleanup owns both the storage subscription and pending read.
 */
export function observeStarterPrompts(
  mode: ChatMode,
  onChange: (prompts: StarterPrompt[]) => void,
  onError: (error: unknown) => void,
): () => void {
  let active = true
  let changed = false
  const store = stores[mode]
  // Subscribe before reading. A slow initial read must not replace a newer
  // storage event emitted by a save in this or another tab.
  const unwatch = store.watch((value) => {
    changed = true
    if (active) onChange(resolveStarterPrompts(mode, value))
  })
  store.getValue().then(
    (value) => {
      if (active && !changed) onChange(resolveStarterPrompts(mode, value))
    },
    (error) => {
      if (active && !changed) onError(error)
    },
  )
  return () => {
    active = false
    unwatch()
  }
}

export async function saveStarterPrompts(
  mode: ChatMode,
  prompts: StarterPrompt[],
): Promise<void> {
  await stores[mode].setValue(starterPromptsSchema.parse(prompts))
}
