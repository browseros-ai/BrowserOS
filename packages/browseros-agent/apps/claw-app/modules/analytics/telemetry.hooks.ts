/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * react-query-kit factories for the shared telemetry state. The server
 * owns the canonical analytics UUID and the user's consent choice; the
 * cockpit reads them here so its own posthog-js shares one identity and
 * the opt-out toggle governs both surfaces.
 */

import type { TelemetryState } from '@browseros/claw-api'
import { createMutation, createQuery } from 'react-query-kit'
import { apiClient } from '@/modules/api/client'

export const TELEMETRY_QUERY_KEY = ['system', 'telemetry'] as const
const TELEMETRY_REFRESH_INTERVAL_MS = 30_000

export const useTelemetryState = createQuery<TelemetryState>({
  queryKey: TELEMETRY_QUERY_KEY,
  fetcher: async () => (await apiClient()).getTelemetry(),
  // The sidecar can restart/upgrade independently of an open cockpit. Refresh
  // its canonical ID and effective consent without requiring a tab reload.
  // Background tabs refresh on focus; visible tabs also discover quick restarts
  // that never produced a browser-level offline/reconnect event.
  staleTime: TELEMETRY_REFRESH_INTERVAL_MS,
  refetchInterval: TELEMETRY_REFRESH_INTERVAL_MS,
  refetchOnWindowFocus: 'always',
  refetchOnReconnect: 'always',
})

export const useSetTelemetryConsent = createMutation<
  TelemetryState,
  { consent: boolean }
>({
  mutationFn: async ({ consent }) =>
    (await apiClient()).updateTelemetry({
      updateTelemetryRequest: { consent },
    }),
  // Polling can read the old consent before PUT but return after it. Cancel
  // those reads before the caller publishes the mutation response to the cache;
  // otherwise an old enabled=true response could resume capture after opt-out.
  // This runs after PUT so it also catches polls started during the mutation.
  onSuccess: async (_state, _variables, _result, { client }) => {
    await client.cancelQueries({ queryKey: TELEMETRY_QUERY_KEY })
  },
})
