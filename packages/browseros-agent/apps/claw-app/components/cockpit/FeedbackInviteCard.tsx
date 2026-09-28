/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Invites the most active installations to a feedback call. The server decides
 * who is eligible and enforces that this is offered once ever; the card asks,
 * shows, and reports back what happened.
 */

import { useQueryClient } from '@tanstack/react-query'
import { CalendarCheck, X } from 'lucide-react'
import { useEffect, useRef, useState } from 'react'
import { Button } from '@/components/ui/button'
import { AnalyticsEvent, track } from '@/modules/analytics/events'
import { isCapturing } from '@/modules/analytics/posthog'
import {
  useFeedbackInvitation,
  useRecordFeedbackInvite,
} from '@/modules/api/feedback.hooks'

/**
 * Remembers that the impression has already been counted for this browser
 * profile. Only the analytics event is deduplicated here; whether the card
 * appears at all stays the server's answer, so losing this key costs at most a
 * repeated impression and never a missed invitation.
 */
const SHOWN_TRACKED_KEY = 'feedbackInviteShownTracked'

/**
 * Remembers that the reader dismissed the card, so the other cockpit tabs they
 * already have open stop showing it too. The card now returns on every load
 * until dismissed, so without this a dismissal in one tab leaves every other
 * open tab still offering it until each one reloads.
 *
 * Only ever suppresses. The server remains the authority on whether anyone is
 * invited, so a cleared key costs at most one more appearance and can never
 * reveal an invitation the server has refused.
 */
const DISMISSED_KEY = 'feedbackInviteDismissed'

function dismissedHere(): boolean {
  try {
    return localStorage.getItem(DISMISSED_KEY) === 'true'
  } catch {
    return false
  }
}

function rememberDismissal(): void {
  try {
    localStorage.setItem(DISMISSED_KEY, 'true')
  } catch {
    // Without storage access the other open tabs keep showing it until reload.
  }
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
 * mid-view; it lives on that until they dismiss it.
 */
type InviteState =
  | { phase: 'waiting' }
  | { phase: 'showing'; bookUrl: string }
  | { phase: 'dismissed' }

export function FeedbackInviteCard() {
  const queryClient = useQueryClient()
  const invitation = useFeedbackInvitation()
  const record = useRecordFeedbackInvite()
  const [state, setState] = useState<InviteState>({ phase: 'waiting' })
  const appeared = useRef(false)
  const booked = useRef(false)

  const offered =
    invitation.data?.eligible === true && !dismissedHere()
      ? invitation.data.bookUrl
      : undefined
  const report = record.mutate

  // The impression belongs to the card appearing, and there is no user action
  // to hang that on. The ref keeps it to the first appearance in this page.
  //
  // The card now returns on every cockpit load until it is dismissed, and the
  // cockpit is the new tab page, so tracking every appearance would report
  // thousands of impressions for one reader and make the funnel's denominator
  // meaningless. The analytics event is counted once per profile; the server is
  // told every time, because it is what holds the first-seen timestamp.
  useEffect(() => {
    if (!offered || appeared.current) return
    appeared.current = true
    setState({ phase: 'showing', bookUrl: offered })
    // The marker is only written once the event has somewhere to go. Analytics
    // readiness is settled by its own request, so the invitation can arrive
    // first; marking the impression then would drop it and never retry, leaving
    // the profile out of the count for good.
    if (!impressionAlreadyCounted() && isCapturing()) {
      rememberImpression()
      track(AnalyticsEvent.FeedbackInviteShown)
    }
    report({ outcome: 'shown' })
  }, [offered, report])

  if (state.phase !== 'showing') return null
  const { bookUrl } = state

  // The reply to a recorded outcome is the invitation's new state, so it is
  // written straight into the cache rather than invalidated for a refetch that
  // would ask the same question again.
  const recordOutcome = (outcome: 'clicked' | 'dismissed') => {
    report(
      { outcome },
      {
        onSuccess: (settled) => {
          queryClient.setQueryData(useFeedbackInvitation.getKey(), settled)
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
    rememberDismissal()
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
