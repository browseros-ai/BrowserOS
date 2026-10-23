/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { useEffect, useState, useSyncExternalStore } from 'react'
import type { ChatTurn } from './onboarding-chat'

const BEAT_MS = 260
const TYPING_MS = 600

const REDUCED_MOTION_QUERY = '(prefers-reduced-motion: reduce)'

function subscribeReducedMotion(onChange: () => void) {
  const query = window.matchMedia(REDUCED_MOTION_QUERY)
  query.addEventListener('change', onChange)
  return () => query.removeEventListener('change', onChange)
}

export function usePrefersReducedMotion(): boolean {
  return useSyncExternalStore(
    subscribeReducedMotion,
    () => window.matchMedia(REDUCED_MOTION_QUERY).matches,
    () => false,
  )
}

export interface ChatReveal {
  revealed: ReadonlySet<string>
  /** The turn showing neo's typing dots, if any. */
  typingTurnId: string | null
  /** Every bubble is on screen, so the reply dock may appear. */
  settled: boolean
}

export interface RevealPlan {
  /** Bubbles on screen in this render. */
  shown: ReadonlySet<string>
  /** The first bubble still to be typed, if any. */
  next: { id: string; turnId: string } | null
}

/**
 * Which bubbles are on screen, given those already typed. Replies and the
 * Keychain heads-up are instant: they show in the same render that adds them
 * (as long as nothing before them is still being typed), with no effect in
 * between. The import click relies on that: it commits this render with
 * flushSync, then sends the request, so the heads-up precedes macOS's prompt.
 */
export function revealPlan(
  turns: readonly ChatTurn[],
  typed: ReadonlySet<string>,
  reducedMotion: boolean,
): RevealPlan {
  const shown = new Set<string>()
  let next: RevealPlan['next'] = null
  for (const turn of turns) {
    for (const bubble of turn.bubbles) {
      const instant =
        reducedMotion || turn.from === 'you' || bubble.kind === 'keychain'
      if (typed.has(bubble.id) || (instant && !next)) {
        shown.add(bubble.id)
        continue
      }
      next ??= { id: bubble.id, turnId: turn.id }
    }
  }
  return { shown, next }
}

/**
 * Paces the transcript like someone typing: each new neo bubble waits a beat,
 * shows typing dots, then lands. The transcript itself is derived fresh from
 * every Chromium snapshot; only this pacing is stateful, keyed by bubble id, so
 * a bubble whose text changes in place (import progress) never re-types.
 */
export function useChatReveal(turns: readonly ChatTurn[]): ChatReveal {
  const reducedMotion = usePrefersReducedMotion()
  const [typed, setTyped] = useState<ReadonlySet<string>>(() => new Set())
  const [typing, setTyping] = useState(false)
  const { shown, next } = revealPlan(turns, typed, reducedMotion)
  const nextId = next?.id

  useEffect(() => {
    if (!nextId) return
    const typingTimer = window.setTimeout(() => setTyping(true), BEAT_MS)
    const revealTimer = window.setTimeout(() => {
      setTyping(false)
      setTyped((prev) => new Set(prev).add(nextId))
    }, BEAT_MS + TYPING_MS)
    return () => {
      window.clearTimeout(typingTimer)
      window.clearTimeout(revealTimer)
      setTyping(false)
    }
  }, [nextId])

  const pendingTurn = turns.at(-1)?.pending ? turns.at(-1) : undefined
  return {
    revealed: shown,
    typingTurnId:
      typing && next ? next.turnId : !next ? (pendingTurn?.id ?? null) : null,
    settled: !next && !pendingTurn,
  }
}
