/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * react-query-kit factories for the v2 MCP page's per-harness
 * Connect / Disconnect buttons.
 */

import { createMutation, createQuery } from 'react-query-kit'
import type { Harness } from '@/components/harness/harness.types'
import { api } from './client'
import { parseResponse } from './parseResponse'

export interface ConnectionState {
  harness: Harness
  installed: boolean
  configPath?: string
  /** Stable agent-mcp-manager id; null for BrowserOS-internal harnesses. */
  agentId: string | null
  message: string
}

interface ConnectionsResponse {
  connections: ConnectionState[]
}

export const useBrowserosConnections = createQuery<ConnectionsResponse>({
  queryKey: ['cockpit', 'connections'],
  fetcher: async () => {
    const response = await api.connections.$get()
    return parseResponse<ConnectionsResponse>(response)
  },
  refetchInterval: 5000,
})

interface ConnectVariables {
  harness: Harness
}

export const useConnectBrowseros = createMutation<
  ConnectionState,
  ConnectVariables
>({
  mutationFn: async ({ harness }) => {
    const response = await api.connections[':harness'].connect.$post({
      param: { harness },
    })
    return parseResponse<ConnectionState>(response)
  },
})

export const useDisconnectBrowseros = createMutation<
  ConnectionState,
  ConnectVariables
>({
  mutationFn: async ({ harness }) => {
    const response = await api.connections[':harness'].disconnect.$post({
      param: { harness },
    })
    return parseResponse<ConnectionState>(response)
  },
})
