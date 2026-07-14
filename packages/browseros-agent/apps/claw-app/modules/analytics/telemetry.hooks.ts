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

import { createMutation, createQuery } from 'react-query-kit'
import { api } from '@/modules/api/client'
import { parseResponse } from '@/modules/api/parseResponse'

export interface TelemetryState {
  /** Anonymous install UUID shared with the server. */
  distinctId: string
  /** Effective server-side state (informational). */
  enabled: boolean
  /** The user's consent choice; drives the toggle and the extension's capture. */
  consent: boolean
}

export const TELEMETRY_QUERY_KEY = ['system', 'telemetry'] as const

export const useTelemetryState = createQuery<TelemetryState>({
  queryKey: TELEMETRY_QUERY_KEY,
  fetcher: async () => {
    const response = await api.system.telemetry.$get()
    return parseResponse<TelemetryState>(response)
  },
  // Identity + consent change rarely (only via the toggle, which
  // invalidates this query), so don't poll.
  staleTime: Number.POSITIVE_INFINITY,
})

export const useSetTelemetryConsent = createMutation<
  TelemetryState,
  { consent: boolean }
>({
  mutationFn: async ({ consent }) => {
    const response = await api.system.telemetry.$post({ json: { consent } })
    return parseResponse<TelemetryState>(response)
  },
})
