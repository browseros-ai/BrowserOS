/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Invites the most active installations to a feedback call. The server decides
 * who is eligible and snoozes the invitation for a few days after the reader
 * books or declines; the card asks, shows, and reports back what happened.
 */

import { useQueryClient } from '@tanstack/react-query'
import { ArrowRight, CalendarCheck } from 'lucide-react'
import { useEffect, useRef, useState, useSyncExternalStore } from 'react'
import { Button } from '@/components/motion/button/base'
import { AnalyticsEvent, track } from '@/modules/analytics/events'
import {
  isCapturing,
  subscribeToCaptureState,
} from '@/modules/analytics/posthog'
import {
  useFeedbackInvitation,
  useRecordFeedbackInvite,
} from '@/modules/api/feedback.hooks'
import { toastApiError } from '@/modules/toast/toast'
import { PromoCard } from './PromoCard'

/**
 * Remembers that the impression has already been counted for this browser
 * profile. Only the analytics event is deduplicated here; whether the card
 * appears at all stays the server's answer, so losing this key costs at most a
 * repeated impression and never a missed invitation.
 */
const SHOWN_TRACKED_KEY = 'feedbackInviteShownTracked'

/**
 * Fences stale eligible query results after a server-confirmed answer. The
 * timestamp is compared with when the query's request started, so a request
 * already in flight when the answer was saved cannot bring the card back, while
 * any request made afterwards is the server's word and always wins. Browser
 * storage never becomes an eligibility source, which is also what lets the card
 * return once the server's snooze runs out.
 */
const ANSWERED_AT_KEY = 'feedbackInviteDismissedAt:v1'

function readAnsweredAt(): number | null {
  try {
    const value = Number(localStorage.getItem(ANSWERED_AT_KEY))
    return Number.isFinite(value) && value > 0 ? value : null
  } catch {
    return null
  }
}

function rememberAnswer(): void {
  try {
    localStorage.setItem(ANSWERED_AT_KEY, String(Date.now()))
  } catch {
    // The server still holds the snooze when storage is unavailable.
  }
}

function subscribeToAnswers(listener: () => void): () => void {
  const onStorage = (event: StorageEvent) => {
    if (event.key === ANSWERED_AT_KEY) listener()
  }
  window.addEventListener('storage', onStorage)
  return () => window.removeEventListener('storage', onStorage)
}

function impressionAlreadyCounted(): boolean {
  try {
    return localStorage.getItem(SHOWN_TRACKED_KEY) === 'true'
  } catch {
    return false
  }
}

function rememberImpression(): void {
  try {
    localStorage.setItem(SHOWN_TRACKED_KEY, 'true')
  } catch {
    // A new tab without storage access simply counts the impression again.
  }
}

/**
 * What this page load has decided to show. The card is copied here once so it
 * cannot be pulled out from under the reader by the query answering again
 * mid-view; it lives on that until they book or decline.
 */
type InviteState =
  | { phase: 'waiting' }
  | { phase: 'showing'; bookUrl: string }
  | { phase: 'answered' }

export interface FeedbackOffer {
  /** The booking link while the server says this installation is invited. */
  bookUrl?: string
  /** True once an answer saved after the last question fences the invitation. */
  fencedByNewerAnswer: boolean
}

/** Whether the server currently invites this installation, after the answer fence. */
export function useFeedbackOffer(): FeedbackOffer {
  const invitation = useFeedbackInvitation()
  const answeredAt = useSyncExternalStore(
    subscribeToAnswers,
    readAnsweredAt,
    () => null,
  )
  const askedAt = invitation.data?.requestedAt ?? invitation.dataUpdatedAt
  const fencedByNewerAnswer = answeredAt !== null && answeredAt >= askedAt
  const bookUrl =
    invitation.data?.eligible === true && !fencedByNewerAnswer
      ? invitation.data.bookUrl
      : undefined
  return { bookUrl, fencedByNewerAnswer }
}

export function FeedbackInviteCard({ onClosed }: { onClosed?: () => void }) {
  const queryClient = useQueryClient()
  const record = useRecordFeedbackInvite()
  const { bookUrl: offered, fencedByNewerAnswer } = useFeedbackOffer()
  const capturing = useSyncExternalStore(
    subscribeToCaptureState,
    isCapturing,
    () => false,
  )
  const [state, setState] = useState<InviteState>({ phase: 'waiting' })
  const appeared = useRef(false)

  const report = record.mutate

  // The impression belongs to the card appearing, and there is no user action
  // to hang that on. The ref keeps it to the first appearance in this page.
  //
  // The card returns on every cockpit load outside a snooze, and the cockpit is
  // the new tab page, so tracking every appearance would report
  // thousands of impressions for one reader and make the funnel's denominator
  // meaningless. The analytics event is counted once per profile; the server is
  // told every time, because it is what holds the first-seen timestamp.
  useEffect(() => {
    if (!offered || appeared.current) return
    appeared.current = true
    setState({ phase: 'showing', bookUrl: offered })
    report({ outcome: 'shown' })
  }, [offered, report])

  // PostHog readiness changes independently of invitation eligibility. A
  // subscribed snapshot lets the same mounted card count its impression when
  // capture comes online, without polling or waiting for another page load.
  useEffect(() => {
    if (state.phase !== 'showing' || !capturing || impressionAlreadyCounted()) {
      return
    }
    rememberImpression()
    track(AnalyticsEvent.FeedbackInviteShown)
  }, [capturing, state.phase])

  if (state.phase !== 'showing' || fencedByNewerAnswer) return null
  const { bookUrl } = state

  // The reply to a recorded outcome is the invitation's new state, so it is
  // written straight into the cache rather than invalidated for a refetch that
  // would ask the same question again. Booking and declining both put the card
  // away; the server brings it back after a few days, because opening the
  // booking page is not the same as booking. Once the answer is saved the card
  // may appear again on this mount, so a long-open cockpit picks up the
  // server's next eligible answer instead of waiting for a reload.
  const answer = (outcome: 'clicked' | 'dismissed') => {
    track(
      outcome === 'clicked'
        ? AnalyticsEvent.FeedbackInviteClicked
        : AnalyticsEvent.FeedbackInviteDismissed,
    )
    setState({ phase: 'answered' })
    onClosed?.()
    report(
      { outcome },
      {
        onSuccess: (settled) => {
          queryClient.setQueryData(useFeedbackInvitation.getKey(), settled)
          rememberAnswer()
          appeared.current = false
        },
        onError: () => {
          toastApiError(
            'Could not save your response. The invitation may appear again.',
          )
        },
      },
    )
  }

  const handleBook = () => {
    chrome.tabs.create({ url: bookUrl })
    answer('clicked')
  }

  const handleDecline = () => answer('dismissed')

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
    <PromoCard
      aria-label="Feedback call invitation"
      icon={<CalendarCheck />}
      title="You’re one of our most active users"
      description="Tell us what you use neo for and what we should fix."
      onDismiss={onDecline}
      action={
        <>
          <Button
            size="sm"
            variant="ghost"
            onClick={onBook}
            className="h-8 px-2.5 font-semibold text-[13.5px] text-accent-ink hover:bg-card/70 hover:text-accent-ink"
          >
            Book a 15 minute chat
            <ArrowRight aria-hidden className="size-[15px]" />
          </Button>
          <Button
            size="sm"
            variant="ghost"
            onClick={onDecline}
            className="h-8 px-2.5 font-semibold text-[13.5px]"
          >
            No thanks
          </Button>
        </>
      }
    />
  )
}
