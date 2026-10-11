/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Server-owned settings for the tools BrowserOS neo offers to agents, such as
 * whether they may ask a person for help. Callers publish a save's response to
 * the cache under `useAgentSettings.getKey()`.
 */

import type {
  AgentSettings,
  UpdateAgentSettingsRequest,
} from '@browseros/claw-api'
import { createMutation, createQuery } from 'react-query-kit'
import { apiClient } from './client'

export const useAgentSettings = createQuery<AgentSettings>({
  queryKey: ['api', 'settings', 'agent'],
  fetcher: async () => (await apiClient()).getAgentSettings(),
})

export const useUpdateAgentSettings = createMutation<
  AgentSettings,
  UpdateAgentSettingsRequest
>({
  mutationFn: async (body) => (await apiClient()).updateAgentSettings(body),
  // A refetch that read the old value can answer after the save. Cancel it
  // before the caller publishes the saved value to the cache.
  onSuccess: async (_saved, _body, _result, { client }) => {
    await client.cancelQueries({ queryKey: useAgentSettings.getKey() })
  },
})
