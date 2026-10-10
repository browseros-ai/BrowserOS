/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * react-query-kit factories for the server-owned agent tool settings, such as
 * whether agents are offered the human-help tools.
 */

import type { AgentSettings } from '@browseros/claw-api'
import { createMutation, createQuery } from 'react-query-kit'
import { apiClient } from './client'

const AGENT_SETTINGS_QUERY_KEY = ['settings', 'agent'] as const

export const useAgentSettings = createQuery<AgentSettings>({
  queryKey: AGENT_SETTINGS_QUERY_KEY,
  fetcher: async () => (await apiClient()).getAgentSettings(),
})

export const useUpdateAgentSettings = createMutation<
  AgentSettings,
  AgentSettings
>({
  mutationFn: async (settings) =>
    (await apiClient()).updateAgentSettings(settings),
  onSuccess: (settings, _variables, _result, { client }) => {
    client.setQueryData(AGENT_SETTINGS_QUERY_KEY, settings)
  },
})
