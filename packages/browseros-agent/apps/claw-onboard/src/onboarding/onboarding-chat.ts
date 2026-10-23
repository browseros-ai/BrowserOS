/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * The onboarding conversation as data. neo walks the person through setup in
 * a short chat, and the person only ever presses reply buttons.
 *
 *   Hi ──Set me up──► Logins (chips from the picked source)
 *    │                  │──Bring my logins──► [Mac heads-up] ► progress ► Done
 *    │                  └──Not now──┐                                    │
 *    └──Reconnect───────────────────┴──► "Getting your cockpit ready…" ◄─┘
 *
 * `buildOnboardingChat` is the whole flow: it turns the Chromium snapshot and
 * the person's local choices into the transcript and the reply dock. The view
 * only renders it and maps dock actions to bridge calls, so every branch
 * (Windows sources without logins, failures, setup retry) is tested here
 * without React. The page ends after the import; the cockpit carries the chat
 * on with the connected apps and the demo once native has prepared it.
 */

import type {
  BrowserOSImportItem,
  BrowserOSImportSource,
  BrowserOSOnboardingState,
  BrowserOSStartImportRequest,
} from './browseros-onboarding-api'
import {
  browserShortName,
  type ClientPlatform,
  defaultImportItemsForSource,
  type ImportChip,
  importChipsFor,
  importDoneLine,
  includesLogins,
  needsMacKeychainPermission,
  offeredSource,
  sanitizeImportSelection,
  startImportRequestFor,
} from './import-selection'

/** What the person has done on this page; Chromium owns everything else. */
export interface ChatChoices {
  step: 'hi' | 'logins'
  /** The reply that ended the chat before an import ("Reconnect", "Not now"). */
  exitReply?: string
  /** Null follows Chromium's first source, so a refreshed list needs no sync. */
  sourceId: string | null
  /** Null means the picked source's defaults; reset whenever the source changes. */
  items: BrowserOSImportItem[] | null
  /** Set on the import click, so the person's reply shows before native answers. */
  importRequested: boolean
}

export const initialChatChoices: ChatChoices = {
  step: 'hi',
  sourceId: null,
  items: null,
  importRequested: false,
}

export type ChatChoicesEvent =
  | { type: 'start' }
  | { type: 'exit'; reply: string }
  | { type: 'pickSource'; sourceId: string }
  | {
      type: 'toggleItem'
      source: BrowserOSImportSource
      item: BrowserOSImportItem
    }
  | { type: 'importRequested' }

export function chatChoicesReducer(
  choices: ChatChoices,
  event: ChatChoicesEvent,
): ChatChoices {
  switch (event.type) {
    case 'start':
      return { ...choices, step: 'logins' }
    case 'exit':
      return { ...choices, exitReply: event.reply }
    case 'pickSource':
      if (event.sourceId === choices.sourceId) return choices
      return { ...choices, sourceId: event.sourceId, items: null }
    case 'toggleItem': {
      const current = selectedItemsFor(event.source, choices)
      const items = current.includes(event.item)
        ? current.filter((item) => item !== event.item)
        : sanitizeImportSelection(event.source, [...current, event.item])
      return { ...choices, sourceId: event.source.id, items }
    }
    case 'importRequested':
      return { ...choices, importRequested: true }
  }
}

function selectedSourceFor(
  sources: readonly BrowserOSImportSource[],
  choices: ChatChoices,
): BrowserOSImportSource | undefined {
  return sources.find((source) => source.id === choices.sourceId) ?? sources[0]
}

function selectedItemsFor(
  source: BrowserOSImportSource,
  choices: ChatChoices,
): BrowserOSImportItem[] {
  if (choices.items === null || choices.sourceId !== source.id) {
    return defaultImportItemsForSource(source)
  }
  return sanitizeImportSelection(source, choices.items)
}

export type ChatBubble =
  | { id: string; kind: 'hero' }
  | { id: string; kind: 'scene'; live: boolean }
  | { id: string; kind: 'text'; text: string; tone?: 'display' | 'quiet' }
  | {
      id: string
      kind: 'chips'
      browser: string
      chips: Array<ImportChip & { on: boolean }>
      editable: boolean
    }
  | { id: string; kind: 'keychain' }
  | { id: string; kind: 'progress'; text: string; value: number }
  | { id: string; kind: 'working'; text: string }

export interface ChatTurn {
  id: string
  from: 'neo' | 'you'
  bubbles: ChatBubble[]
  /** neo is still thinking (e.g. looking for browsers): show typing, no bubble. */
  pending?: boolean
}

export type ChatAction =
  | 'start'
  | 'reconnect'
  | 'skip'
  | 'import'
  | 'refresh'
  | 'retrySetup'

export type DockItem =
  | {
      kind: 'primary' | 'secondary' | 'quiet'
      label: string
      /** Muted lead-in before a quiet label: "Already set up? Reconnect". */
      prefix?: string
      action: ChatAction
      disabled?: boolean
    }
  | {
      kind: 'profile'
      sources: readonly BrowserOSImportSource[]
      selectedId: string
    }

export interface OnboardingChat {
  /** Lit dashes in the top progress strip: Hi, Logins, then setup. */
  progress: 1 | 2 | 3
  turns: ChatTurn[]
  dock: DockItem[]
}

export interface OnboardingChatInput {
  state: BrowserOSOnboardingState
  choices: ChatChoices
  platform: ClientPlatform
}

/** The picked source as this platform may offer it (no logins on Windows).
 * The transcript, chip toggles and the import request all go through here. */
export function chatSource({
  state,
  choices,
  platform,
}: OnboardingChatInput): BrowserOSImportSource | undefined {
  const source = selectedSourceFor(state.sources, choices)
  return source && offeredSource(source, platform)
}

/** What the import click sends; null when nothing is selected. */
export function chatImportRequest(
  input: OnboardingChatInput,
): BrowserOSStartImportRequest | null {
  const source = chatSource(input)
  if (!source) return null
  return startImportRequestFor(source, selectedItemsFor(source, input.choices))
}

type ImportOutcome = 'importing' | 'succeeded' | 'failed'

/**
 * Native can report a failure under any status (`SendFailure("ready", …)`),
 * and `completed` after setup keeps the import result only in `results`. A
 * requested import that native has not answered yet counts as importing: the
 * click commits that render (with the Mac heads-up) before it sends the
 * request, so the heads-up is on screen before macOS can prompt.
 */
function importOutcomeFor(
  state: BrowserOSOnboardingState,
  choices: ChatChoices,
): ImportOutcome | null {
  if (!choices.importRequested) return null
  if (state.status === 'succeeded') return 'succeeded'
  if (state.status === 'failed' || state.error) return 'failed'
  if (state.status === 'completed') {
    const result = state.results?.[0]?.status
    if (result === 'succeeded' || result === 'failed') return result
  }
  return 'importing'
}

const HI_BUBBLES = (live: boolean): ChatBubble[] => [
  { id: 'hi-hero', kind: 'hero' },
  { id: 'hi-line', kind: 'text', text: 'I’m the browser your AI uses.' },
  { id: 'hi-scene', kind: 'scene', live },
]

function pushNeo(turns: ChatTurn[], ...bubbles: ChatBubble[]) {
  const last = turns.at(-1)
  if (last?.from === 'neo' && !last.pending) {
    last.bubbles.push(...bubbles)
    return
  }
  turns.push({ id: bubbles[0]?.id ?? 'neo', from: 'neo', bubbles })
}

function pushYou(turns: ChatTurn[], id: string, text: string) {
  turns.push({ id, from: 'you', bubbles: [{ id, kind: 'text', text }] })
}

function progressText(
  state: BrowserOSOnboardingState,
  items: readonly BrowserOSImportItem[],
) {
  const done = state.progress?.completedItems.length ?? 0
  const total = state.progress?.totalItems ?? items.length
  const lead = includesLogins(items)
    ? 'Bringing your logins…'
    : 'Bringing them over…'
  return {
    text: total > 0 ? `${lead} ${done} of ${total}` : lead,
    value: total > 0 ? Math.min(1, done / total) : 0,
  }
}

const HI_DOCK: DockItem[] = [
  {
    kind: 'quiet',
    prefix: 'Already set up?',
    label: 'Reconnect',
    action: 'reconnect',
  },
  { kind: 'primary', label: 'Set me up', action: 'start' },
]

function isSetupActive(state: BrowserOSOnboardingState): boolean {
  return Boolean(state.setupState && state.setupState !== 'idle')
}

/** neo's ask and the chips; returns the logins dock while choices are open. */
function addLogins(turns: ChatTurn[], input: OnboardingChatInput): DockItem[] {
  const { state, choices, platform } = input
  pushYou(turns, 'you-start', 'Set me up')
  const source = chatSource(input)
  const isDetecting = state.status === 'detecting' || state.status === 'idle'

  if (!source && isDetecting) {
    turns.push({ id: 'logins-wait', from: 'neo', bubbles: [], pending: true })
    return []
  }
  if (!source) {
    pushNeo(turns, {
      id: 'logins-none',
      kind: 'text',
      text: 'I didn’t find another browser to bring things from.',
    })
    return [
      { kind: 'quiet', label: 'Look again', action: 'refresh' },
      { kind: 'primary', label: 'Continue', action: 'skip' },
    ]
  }

  const outcome = importOutcomeFor(state, choices)
  const items = selectedItemsFor(source, choices)
  const browser = browserShortName(source)
  const locked =
    outcome === 'importing' ||
    outcome === 'succeeded' ||
    Boolean(choices.exitReply) ||
    isSetupActive(state)
  pushNeo(
    turns,
    {
      id: 'logins-ask',
      kind: 'text',
      tone: 'display',
      // Without logins (always on Windows, or a profile with none saved) neo
      // asks the plainer question: there is nothing to sign the AI in with.
      text: includesLogins(source.supportedItems)
        ? 'Want your AI to use your logins?'
        : `Want your ${browser} things here?`,
    },
    {
      id: 'logins-chips',
      kind: 'chips',
      browser,
      chips: importChipsFor(source).map((chip) => ({
        ...chip,
        on: items.includes(chip.item),
      })),
      editable: !locked,
    },
  )

  const importLabel = includesLogins(items)
    ? 'Bring my logins'
    : 'Bring them over'
  if (outcome) {
    pushYou(turns, 'you-import', importLabel)
    if (needsMacKeychainPermission(platform === 'mac', source, items)) {
      pushNeo(turns, { id: 'import-keychain', kind: 'keychain' })
    }
    addImportResult(turns, state, outcome, items)
  }
  if (locked) return []
  return [
    { kind: 'quiet', label: 'Not now', action: 'skip' },
    { kind: 'profile', sources: state.sources, selectedId: source.id },
    {
      kind: 'primary',
      label: outcome === 'failed' ? 'Try again' : importLabel,
      action: 'import',
      disabled: items.length === 0 || isDetecting,
    },
  ]
}

/** Counts are Chromium's import categories; native reports no site counts. */
function addImportResult(
  turns: ChatTurn[],
  state: BrowserOSOnboardingState,
  outcome: ImportOutcome,
  items: readonly BrowserOSImportItem[],
) {
  if (outcome === 'failed') {
    pushNeo(turns, {
      id: 'import-failed',
      kind: 'text',
      text: 'I couldn’t bring them over.',
    })
    if (state.error?.message) {
      pushNeo(turns, {
        id: 'import-error',
        kind: 'text',
        tone: 'quiet',
        text: state.error.message,
      })
    }
    return
  }
  pushNeo(turns, {
    id: 'import-progress',
    kind: 'progress',
    ...(outcome === 'succeeded'
      ? {
          // Only what Chromium reported done; without progress, claim nothing.
          text: importDoneLine(state.progress?.completedItems ?? []),
          value: 1,
        }
      : progressText(state, items)),
  })
}

/** Native prepares the product after COMPLETE and navigates on success. */
function addSetup(turns: ChatTurn[], state: BrowserOSOnboardingState) {
  const failed = state.setupState === 'failed'
  pushNeo(turns, {
    id: failed ? 'setup-failed' : 'setup',
    kind: failed ? 'text' : 'working',
    text: failed
      ? 'Setup didn’t finish. Let’s try that again.'
      : 'Getting your cockpit ready…',
  })
  return failed
    ? [{ kind: 'primary', label: 'Try again', action: 'retrySetup' } as const]
    : []
}

export function buildOnboardingChat(
  input: OnboardingChatInput,
): OnboardingChat {
  const { state, choices } = input
  const setupActive = isSetupActive(state)
  const onHi = choices.step === 'hi' && !choices.exitReply && !setupActive
  const turns: ChatTurn[] = [
    { id: 'hi', from: 'neo', bubbles: HI_BUBBLES(onHi) },
  ]
  if (onHi) return { progress: 1, turns, dock: HI_DOCK }

  let dock = choices.step === 'logins' ? addLogins(turns, input) : []
  if (choices.exitReply) {
    pushYou(turns, 'you-exit', choices.exitReply)
    dock = []
  }
  if (setupActive) dock = addSetup(turns, state)
  return { progress: setupActive ? 3 : 2, turns, dock }
}
