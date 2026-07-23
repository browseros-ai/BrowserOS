/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * react-query-kit factories for the shared telemetry state. The server
 * owns the anonymous install id and the user's consent choice; the
 * cockpit reads them here so its own posthog-js shares one identity and
 * the opt-out toggle governs both surfaces.
 */

import type { TelemetryState } from '@browseros/claw-api'
import { createMutation, createQuery } from 'react-query-kit'
import { apiClient } from '@/modules/api/client'

export const TELEMETRY_QUERY_KEY = ['system', 'telemetry'] as const

export const useTelemetryState = createQuery<TelemetryState>({
  queryKey: TELEMETRY_QUERY_KEY,
  fetcher: async () => (await apiClient()).getTelemetry(),
  // Identity + consent change rarely (only via the toggle, which
  // invalidates this query), so don't poll.
  staleTime: Number.POSITIVE_INFINITY,
})

export const useSetTelemetryConsent = createMutation<
  TelemetryState,
  { consent: boolean }
>({
  mutationFn: async ({ consent }) =>
    (await apiClient()).updateTelemetry({
      updateTelemetryRequest: { consent },
    }),
})
