/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * The feedback call invitation shown to the most active installations. The
 * server decides who is eligible and snoozes the invitation for a few days
 * after it is booked or declined; this only asks and reports back.
 */

import type {
  FeedbackInvitation,
  FeedbackInviteOutcome,
} from '@browseros/claw-api'
import { createMutation, createQuery } from 'react-query-kit'
import { apiClient } from './client'

const FEEDBACK_INVITATION_STALE_TIME_MS = 30_000

export const useFeedbackInvitation = createQuery<FeedbackInvitation>({
  queryKey: ['api', 'feedback', 'invitation'],
  fetcher: async () => (await apiClient()).getFeedbackInvitation(),
  staleTime: FEEDBACK_INVITATION_STALE_TIME_MS,
  refetchOnMount: 'always',
})

// Mutations default to no retries. A lost outcome is not free here: the
// impression would be counted again on the next cockpit load, and a booked or
// declined invitation would come back before its snooze, because the server is the authority on both and
// never heard. Every outcome write is idempotent, so retrying is safe.
export const useRecordFeedbackInvite = createMutation<
  FeedbackInvitation,
  { outcome: FeedbackInviteOutcome }
>({
  mutationFn: async ({ outcome }) =>
    (await apiClient()).recordFeedbackInvite({ outcome }),
  retry: 2,
})
