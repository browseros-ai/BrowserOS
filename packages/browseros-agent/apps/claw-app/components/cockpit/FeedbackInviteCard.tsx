/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Invites the most active installations to a feedback call. The server decides
 * who is eligible and keeps the invitation open until dismissal; the card
 * asks, shows, and reports back what happened.
 */

import { useQueryClient } from '@tanstack/react-query'
import { CalendarCheck, X } from 'lucide-react'
import { useEffect, useRef, useState, useSyncExternalStore } from 'react'
import { toast } from 'sonner'
import { Button } from '@/components/ui/button'
import { AnalyticsEvent, track } from '@/modules/analytics/events'
import {
  isCapturing,
  subscribeToCaptureState,
} from '@/modules/analytics/posthog'
import {
  useFeedbackInvitation,
  useRecordFeedbackInvite,
} from '@/modules/api/feedback.hooks'

/**
 * Remembers that the impression has already been counted for this browser
 * profile and round. Only the analytics event is deduplicated here; whether the card
 * appears at all stays the server's answer, so losing this key costs at most a
 * repeated impression and never a missed invitation.
 */
const SHOWN_TRACKED_KEY = 'feedbackInviteShownTracked'

/**
 * Fences stale eligible query results after a server-confirmed dismissal. The
 * timestamp is compared with React Query's dataUpdatedAt, so a newer server
 * answer always wins and browser storage never becomes an eligibility source.
 */
const DISMISSED_AT_KEY = 'feedbackInviteDismissedAt:v1'

function readDismissedAt(): number | null {
  try {
    const value = Number(localStorage.getItem(DISMISSED_AT_KEY))
    return Number.isFinite(value) && value > 0 ? value : null
  } catch {
    return null
  }
}

function rememberDismissal(): void {
  try {
    localStorage.setItem(DISMISSED_AT_KEY, String(Date.now()))
  } catch {
    // The server still holds the durable dismissal when storage is unavailable.
  }
}

function subscribeToDismissals(listener: () => void): () => void {
  const onStorage = (event: StorageEvent) => {
    if (event.key === DISMISSED_AT_KEY) listener()
  }
  window.addEventListener('storage', onStorage)
  return () => window.removeEventListener('storage', onStorage)
}

function impressionKey(round = 1): string {
  return round === 1 ? SHOWN_TRACKED_KEY : `${SHOWN_TRACKED_KEY}:${round}`
}

function impressionAlreadyCounted(round?: number): boolean {
  try {
    return localStorage.getItem(impressionKey(round)) === 'true'
  } catch {
    return false
  }
}

function rememberImpression(round?: number): void {
  try {
    localStorage.setItem(impressionKey(round), 'true')
  } catch {
    // A new tab without storage access simply counts the impression again.
  }
}

/**
 * What this page load has decided to show. The card is copied here once so it
 * cannot be pulled out from under the reader by the query answering again
 * mid-view; it lives on that until they dismiss it.
 */
type InviteState =
  | { phase: 'waiting' }
  | { phase: 'showing'; bookUrl: string; round?: number }
  | { phase: 'dismissed' }

export function FeedbackInviteCard() {
  const queryClient = useQueryClient()
  const invitation = useFeedbackInvitation()
  const record = useRecordFeedbackInvite()
  const dismissedAt = useSyncExternalStore(
    subscribeToDismissals,
    readDismissedAt,
    () => null,
  )
  const capturing = useSyncExternalStore(
    subscribeToCaptureState,
    isCapturing,
    () => false,
  )
  const [state, setState] = useState<InviteState>({ phase: 'waiting' })
  const appeared = useRef(false)
  const booked = useRef(false)

  const fencedByNewerDismissal =
    dismissedAt !== null && dismissedAt >= invitation.dataUpdatedAt
  const offered =
    invitation.data?.eligible === true && !fencedByNewerDismissal
      ? invitation.data.bookUrl
      : undefined
  const report = record.mutate
  const offeredRound = invitation.data?.round

  // The impression belongs to the card appearing, and there is no user action
  // to hang that on. The ref keeps it to the first appearance in this page.
  //
  // The card now returns on every cockpit load until it is dismissed, and the
  // cockpit is the new tab page, so tracking every appearance would report
  // thousands of impressions for one reader and make the funnel's denominator
  // meaningless. The analytics event is counted once per profile and round; the server is
  // told every time, because it is what holds the first-seen timestamp.
  useEffect(() => {
    if (!offered || appeared.current) return
    appeared.current = true
    setState({ phase: 'showing', bookUrl: offered, round: offeredRound })
    report({
      outcome: 'shown',
      ...(offeredRound === undefined ? {} : { round: offeredRound }),
    })
  }, [offered, offeredRound, report])

  // PostHog readiness changes independently of invitation eligibility. A
  // subscribed snapshot lets the same mounted card count its impression when
  // capture comes online, without polling or waiting for another page load.
  useEffect(() => {
    if (
      state.phase !== 'showing' ||
      !capturing ||
      impressionAlreadyCounted(state.round)
    ) {
      return
    }
    rememberImpression(state.round)
    track(AnalyticsEvent.FeedbackInviteShown)
  }, [capturing, state])

  if (state.phase !== 'showing' || fencedByNewerDismissal) return null
  const { bookUrl, round } = state

  // The reply to a recorded outcome is the invitation's new state, so it is
  // written straight into the cache rather than invalidated for a refetch that
  // would ask the same question again.
  const recordOutcome = (outcome: 'clicked' | 'dismissed') => {
    report(
      { outcome, ...(round === undefined ? {} : { round }) },
      {
        onSuccess: (settled) => {
          queryClient.setQueryData(useFeedbackInvitation.getKey(), settled)
          if (outcome === 'dismissed' && settled.eligible === false) {
            rememberDismissal()
          }
        },
        onError: () => {
          if (outcome === 'clicked') booked.current = false
          toast.error(
            outcome === 'dismissed'
              ? 'Could not save your response. The invitation may appear again.'
              : 'Could not record your response. Please try again.',
          )
        },
      },
    )
  }

  // Booking opens a tab and leaves the card alone. Taking it away here would
  // punish the reader for accepting: they land on a booking page, and if they
  // come back to finish later the invitation they agreed to has vanished.
  // Only declining removes it.
  const handleBook = () => {
    if (!booked.current) {
      booked.current = true
      track(AnalyticsEvent.FeedbackInviteClicked)
      recordOutcome('clicked')
    }
    chrome.tabs.create({ url: bookUrl })
  }

  const handleDecline = () => {
    track(AnalyticsEvent.FeedbackInviteDismissed)
    setState({ phase: 'dismissed' })
    recordOutcome('dismissed')
  }

  return (
    <FeedbackInviteCardView onBook={handleBook} onDecline={handleDecline} />
  )
}

export function FeedbackInviteCardView({
  onBook,
  onDecline,
}: {
  onBook: () => void
  onDecline: () => void
}) {
  return (
    <section
      aria-label="Feedback call invitation"
      className="flex items-start gap-3 rounded-[9px] border border-cyanotype-border bg-accent-tint/50 px-4 py-3.5"
    >
      <span className="flex size-8 shrink-0 items-center justify-center rounded-[9px] bg-accent-tint-2 text-cyanotype-blue">
        <CalendarCheck className="size-4" />
      </span>
      <div className="min-w-0 flex-1">
        <p className="font-semibold text-[13px] text-cyanotype-ink leading-5">
          You're one of our most active users
        </p>
        <p className="text-[13px] text-cyanotype-soft leading-5">
          We'd like to hear what you use neo for and what we should fix.
        </p>
        <div className="mt-2.5 flex flex-wrap items-center gap-2">
          <Button
            size="sm"
            onClick={onBook}
            className="h-7 bg-cyanotype-blue px-3 text-[13px] text-on-cyanotype hover:bg-cyanotype-blue-hover"
          >
            Book a 15 minute chat
          </Button>
          <Button
            size="sm"
            variant="ghost"
            onClick={onDecline}
            className="h-7 px-2 text-[13px] text-cyanotype-soft"
          >
            No thanks
          </Button>
        </div>
      </div>
      <button
        type="button"
        onClick={onDecline}
        aria-label="Dismiss"
        className="shrink-0 rounded-sm p-1 text-cyanotype-soft opacity-60 transition-opacity hover:opacity-100"
      >
        <X className="size-3.5" />
      </button>
    </section>
  )
}
