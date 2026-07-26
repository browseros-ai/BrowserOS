/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { CallToolResult, Tool } from '@modelcontextprotocol/sdk/types.js'

export interface ConnectorCatalogItem {
  name: string
  description: string
}

export interface StrataCreateResponse {
  strataServerUrl: string
  strataId: string
  addedServers: string[]
  oauthUrls?: Record<string, string>
  apiKeyUrls?: Record<string, string>
}

export interface UserIntegration {
  name: string
  isAuthenticated: boolean
}

export type KlavisProxyStatus =
  | { state: 'disabled'; reason: 'missing_browseros_id' }
  | { state: 'connecting' }
  | { state: 'ready'; toolCount: number }
  | {
      state: 'retrying'
      attempt: number
      nextRetryMs: number
      error: string
    }
  | { state: 'unavailable'; error: string }
  | { state: 'stopped' }

export interface ConnectorInventory {
  available: ConnectorCatalogItem[]
  connected: UserIntegration[]
  selected: string[]
  proxy: KlavisProxyStatus
}

export interface ConnectorToolScope {
  selectedServerNames?: readonly string[]
}

export interface ConnectorConnectionIntent {
  serverName: string
  strataId: string
  addedServers: string[]
  oauthUrl?: string
  apiKeyUrl?: string
}

export interface SubmitApiKeyInput {
  serverName: string
  apiKey: string
  apiKeyUrl: string
}

export interface KlavisSessionHandle {
  browserosId: string
  tools: Tool[]
  inputSchemas: Map<string, Record<string, never>>
  callTool: (
    name: string,
    args: Record<string, unknown>,
  ) => Promise<CallToolResult>
  close: () => Promise<void>
}
