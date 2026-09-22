/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * The feedback call invitation shown to the most active installations. The
 * server decides who is eligible and enforces that an invitation is offered
 * once ever; this only asks and reports back. Recording an outcome makes the
 * query ineligible, so the mutation invalidates it at the call site via
 * `useFeedbackInvitation.getKey()`.
 */

import type {
  FeedbackInvitation,
  FeedbackInviteOutcome,
} from '@browseros/claw-api'
import { createMutation, createQuery } from 'react-query-kit'
import { apiClient } from './client'

export const useFeedbackInvitation = createQuery<FeedbackInvitation>({
  queryKey: ['api', 'feedback', 'invitation'],
  fetcher: async () => (await apiClient()).getFeedbackInvitation(),
})

export const useRecordFeedbackInvite = createMutation<
  FeedbackInvitation,
  { outcome: FeedbackInviteOutcome }
>({
  mutationFn: async ({ outcome }) =>
    (await apiClient()).recordFeedbackInvite({ outcome }),
})
