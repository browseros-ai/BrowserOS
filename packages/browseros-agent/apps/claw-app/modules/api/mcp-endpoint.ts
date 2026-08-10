/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import {
  BROWSEROS_MCP_SERVER_NAME,
  MCP_PATH,
} from '@browseros/shared/constants/urls'
import {
  apiBaseUrlSourcesFromWindow,
  resolveBrowserOSMcpBaseUrl,
} from './browseros-ports'
import { resolveApiBaseUrlFromSources } from './client.helpers'

export async function resolveMcpBaseUrl(): Promise<string> {
  return resolveBrowserOSMcpBaseUrl(apiBaseUrlSourcesFromWindow())
}

function mcpBaseUrlFallback(): string {
  return resolveApiBaseUrlFromSources(apiBaseUrlSourcesFromWindow())
}

export function buildMcpCliCommand(slug: string): string {
  return `mcp add ${slug}`
}

export function buildCanonicalMcpEndpointUrl(): string {
  return `${mcpBaseUrlFallback()}${MCP_PATH}`
}

export async function resolveCanonicalMcpEndpointUrl(): Promise<string> {
  return `${await resolveMcpBaseUrl()}${MCP_PATH}`
}

export function buildCanonicalMcpCliCommand(): string {
  const url = buildCanonicalMcpEndpointUrl()
  return `claude mcp add ${BROWSEROS_MCP_SERVER_NAME} ${url} --transport http --scope user`
}
