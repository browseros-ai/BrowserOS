/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type {
  ConnectorCatalogItem,
  ConnectorInventory,
  ConnectorToolScope,
  KlavisProxyStatus,
  UserIntegration,
} from './types'

function normalizeServerKey(value: string): string {
  return value.toLowerCase().replace(/[^a-z0-9]+/g, '')
}

export function getAuthUrlForServer(
  authUrlMap: Record<string, string> | undefined,
  serverName: string,
): string | undefined {
  if (!authUrlMap) {
    return undefined
  }
  const directMatch = authUrlMap[serverName]
  if (directMatch) {
    return directMatch
  }
  const targetKey = normalizeServerKey(serverName)
  for (const [key, value] of Object.entries(authUrlMap)) {
    if (normalizeServerKey(key) === targetKey) {
      return value
    }
  }
  return undefined
}

export function selectedServerNames(scope?: ConnectorToolScope): string[] {
  return [...new Set(scope?.selectedServerNames ?? [])]
}

function authenticatedIntegrations(
  integrations: readonly UserIntegration[],
): UserIntegration[] {
  return integrations.filter((integration) => integration.isAuthenticated)
}

export function buildConnectorInventory(input: {
  available: ConnectorCatalogItem[]
  integrations: UserIntegration[]
  proxy: KlavisProxyStatus
  scope?: ConnectorToolScope
}): ConnectorInventory {
  return {
    available: input.available,
    connected: authenticatedIntegrations(input.integrations),
    selected: selectedServerNames(input.scope),
    proxy: input.proxy,
  }
}
