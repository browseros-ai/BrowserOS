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
  SaveJevTokenRequest,
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

/**
 * Stores the decision model credential, which is what offers the pursue tool.
 * Its own mutation rather than a field on the settings save, so saving a switch
 * can never clear a credential. No response ever carries the token back.
 */
export const useSaveJevToken = createMutation<
  AgentSettings,
  SaveJevTokenRequest
>({
  mutationFn: async (body) => (await apiClient()).saveJevToken(body),
  onSuccess: async (_saved, _body, _result, { client }) => {
    await client.cancelQueries({ queryKey: useAgentSettings.getKey() })
  },
})

/** Forgets the stored credential. JEV_TOKEN, if the server has one, applies again. */
export const useRemoveJevToken = createMutation<AgentSettings, void>({
  mutationFn: async () => (await apiClient()).removeJevToken(),
  onSuccess: async (_saved, _body, _result, { client }) => {
    await client.cancelQueries({ queryKey: useAgentSettings.getKey() })
  },
})
