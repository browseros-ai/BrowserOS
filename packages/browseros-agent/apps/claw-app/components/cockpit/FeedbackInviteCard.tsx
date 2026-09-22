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
import {
  useFeedbackInvitation,
  useRecordFeedbackInvite,
} from '@/modules/api/feedback.hooks'

/**
 * What this page load has decided to show. Recording the impression is exactly
 * what makes the server answer 'not eligible', so the card cannot follow the
 * query: it would erase itself the moment it appeared. The invitation is
 * copied here once and the card lives on that until the reader answers.
 */
type InviteState =
  | { phase: 'waiting' }
  | { phase: 'showing'; bookUrl: string }
  | { phase: 'answered' }

export function FeedbackInviteCard() {
  const queryClient = useQueryClient()
  const invitation = useFeedbackInvitation()
  const record = useRecordFeedbackInvite()
  const [state, setState] = useState<InviteState>({ phase: 'waiting' })
  const appeared = useRef(false)

  const offered =
    invitation.data?.eligible === true ? invitation.data.bookUrl : undefined
  const report = record.mutate

  // The impression belongs to the card appearing, and there is no user action
  // to hang that on. The ref keeps it to the first appearance even though the
  // query keeps answering around it.
  useEffect(() => {
    if (!offered || appeared.current) return
    appeared.current = true
    setState({ phase: 'showing', bookUrl: offered })
    track(AnalyticsEvent.FeedbackInviteShown)
    report({ outcome: 'shown' })
  }, [offered, report])

  if (state.phase !== 'showing') return null
  const { bookUrl } = state

  // The reply to a recorded outcome is the invitation's new state, so it is
  // written straight into the cache rather than invalidated for a refetch that
  // would ask the same question again.
  const settle = (outcome: 'clicked' | 'dismissed') => {
    setState({ phase: 'answered' })
    report(
      { outcome },
      {
        onSuccess: (settled) => {
          queryClient.setQueryData(useFeedbackInvitation.getKey(), settled)
        },
      },
    )
  }

  const handleBook = () => {
    track(AnalyticsEvent.FeedbackInviteClicked)
    settle('clicked')
    chrome.tabs.create({ url: bookUrl })
  }

  const handleDecline = () => {
    track(AnalyticsEvent.FeedbackInviteDismissed)
    settle('dismissed')
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
