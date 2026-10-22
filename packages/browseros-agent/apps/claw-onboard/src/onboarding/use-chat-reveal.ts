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

/**
 * Paces the transcript like someone typing: each new neo bubble waits a beat,
 * shows typing dots, then lands. The transcript itself is derived fresh from
 * every Chromium snapshot; only this pacing is stateful, keyed by bubble id, so
 * a bubble whose text changes in place (import progress) never re-types.
 * Replies and the Keychain heads-up land at once: the heads-up has to be read
 * before macOS raises its own prompt. Reduced motion lands everything at once.
 */
export function useChatReveal(turns: readonly ChatTurn[]): ChatReveal {
  const reducedMotion = usePrefersReducedMotion()
  const [revealed, setRevealed] = useState<ReadonlySet<string>>(() => new Set())
  const [typing, setTyping] = useState(false)

  const queue = turns.flatMap((turn) =>
    turn.bubbles.map((bubble) => ({
      id: bubble.id,
      turnId: turn.id,
      instant: turn.from === 'you' || bubble.kind === 'keychain',
    })),
  )
  const next = queue.find((entry) => !revealed.has(entry.id))
  const nextId = next?.id
  const nextInstant = next?.instant ?? false

  useEffect(() => {
    if (!nextId) return
    const reveal = () => setRevealed((prev) => new Set(prev).add(nextId))
    if (reducedMotion || nextInstant) {
      reveal()
      return
    }
    const typingTimer = window.setTimeout(() => setTyping(true), BEAT_MS)
    const revealTimer = window.setTimeout(() => {
      setTyping(false)
      reveal()
    }, BEAT_MS + TYPING_MS)
    return () => {
      window.clearTimeout(typingTimer)
      window.clearTimeout(revealTimer)
      setTyping(false)
    }
  }, [nextId, nextInstant, reducedMotion])

  const pendingTurn = turns.at(-1)?.pending ? turns.at(-1) : undefined
  return {
    revealed,
    typingTurnId:
      typing && next ? next.turnId : !next ? (pendingTurn?.id ?? null) : null,
    settled: !next && !pendingTurn,
  }
}
