/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Per-harness Connect / Disconnect surface for the MCP page's harness
 * cards. The list is polled so state changed outside the app (installs,
 * hand-edited harness configs) shows up without a refresh.
 */

import type { Connection, ConnectionList, Harness } from '@browseros/claw-api'
import { createMutation, createQuery } from 'react-query-kit'
import { apiClient } from './client'

export type ConnectionState = Connection

// Polling is opt-in at the call sites (MCP page, Cockpit) so state changed
// outside the app shows up without a refresh only where that matters.
export const useConnections = createQuery<ConnectionList>({
  queryKey: ['api', 'connections'],
  fetcher: async () => (await apiClient()).listConnections(),
})

interface ConnectionVariables {
  harness: Harness
}

export const useConnectHarness = createMutation<
  Connection,
  ConnectionVariables
>({
  mutationFn: async ({ harness }) =>
    (await apiClient()).connectHarness({ harness }),
})

export const useDisconnectHarness = createMutation<
  Connection,
  ConnectionVariables
>({
  mutationFn: async ({ harness }) =>
    (await apiClient()).disconnectHarness({ harness }),
})
