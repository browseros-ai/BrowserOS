/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * react-query-kit mutation backing the homepage's Watch button.
 * Hits `POST /cockpit/tabs/focus/:agentId`; the server expands the
 * agent's tab group in BrowserOS and activates the group's window.
 */

import { createMutation } from 'react-query-kit'
import { api } from './client'
import { parseResponse } from './parseResponse'

export interface FocusAgentResult {
  ok: boolean
  groupId?: string
  windowId?: number
  reason?: string
}

interface FocusAgentVariables {
  agentId: string
}

export const useFocusAgent = createMutation<
  FocusAgentResult,
  FocusAgentVariables
>({
  mutationFn: async ({ agentId }) => {
    const response = await api.tabs.focus[':agentId'].$post({
      param: { agentId },
    })
    return parseResponse<FocusAgentResult>(response)
  },
})
