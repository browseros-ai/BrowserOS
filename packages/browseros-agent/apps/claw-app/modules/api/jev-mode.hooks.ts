/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Goal-driven browsing: the credential, the pause switch and the run budgets.
 * Every mutation returns the whole state, so one cache write keeps the page
 * and the MCP card in agreement without a refetch.
 */

import type { JevModeState } from '@browseros/claw-api'
import { createMutation, createQuery } from 'react-query-kit'
import { apiClient } from './client'

export const jevModeKey = ['api', 'settings', 'jev-mode'] as const

export const useJevMode = createQuery<JevModeState>({
  queryKey: jevModeKey,
  fetcher: async () => (await apiClient()).getJevMode(),
})

/**
 * Stores a credential, but only if the provider accepts it. A rejection comes
 * back as an error carrying the provider's own reason, which is the whole
 * point: the user learns whether the value is wrong, the account is suspended
 * or the network is blocked.
 */
export const useSaveJevCredential = createMutation<
  JevModeState,
  { credential: string }
>({
  mutationFn: async ({ credential }) =>
    (await apiClient()).updateJevCredential({ credential }),
})

export const useSetJevPaused = createMutation<JevModeState, { paused: boolean }>(
  {
    mutationFn: async ({ paused }) => (await apiClient()).updateJevMode({ paused }),
  },
)

export const useSetJevBudgets = createMutation<
  JevModeState,
  { maxSteps: number; maxSeconds: number }
>({
  mutationFn: async (budgets) => (await apiClient()).updateJevBudgets(budgets),
})

/** Forgets the credential. Separate from pausing, and confirmed in the UI. */
export const useForgetJevCredential = createMutation<JevModeState, void>({
  mutationFn: async () => (await apiClient()).deleteJevCredential(),
})
