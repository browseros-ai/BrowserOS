/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { CLAW_API_PORT_DEFAULT } from '@browseros/shared/constants/ports'
import {
  BROWSEROS_MCP_SERVER_NAME,
  MCP_PATH,
} from '@browseros/shared/constants/urls'
import {
  API_URL_STORAGE_KEY,
  normalizeLoopbackApiRootUrl,
  resolveApiBaseUrlFromSources,
} from './client.helpers'

function fallbackBaseUrl(): string {
  return `http://127.0.0.1:${CLAW_API_PORT_DEFAULT}`
}

/** Resolves the API base URL used by the onboarding MCP CLI snippet. */
function resolveMcpBaseUrl(): string {
  const fallback = fallbackBaseUrl()
  if (typeof window === 'undefined') return fallback

  const query = new URLSearchParams(window.location.search).get('apiUrl')
  const queryBaseUrl = normalizeLoopbackApiRootUrl(query)
  if (queryBaseUrl) {
    try {
      window.sessionStorage.setItem(API_URL_STORAGE_KEY, queryBaseUrl)
    } catch {
      // sessionStorage may reject writes in sandboxed contexts; this call can still use the query URL.
    }
    return queryBaseUrl
  }

  try {
    return resolveApiBaseUrlFromSources({
      query: null,
      stored: window.sessionStorage.getItem(API_URL_STORAGE_KEY),
      launcher: import.meta.env.VITE_BROWSEROS_CLAW_API_URL,
      fallback,
    })
  } catch {
    return resolveApiBaseUrlFromSources({
      query: null,
      stored: null,
      launcher: import.meta.env.VITE_BROWSEROS_CLAW_API_URL,
      fallback,
    })
  }
}

/** Builds the cockpit home URL that standalone onboarding hands back to. */
export function buildCockpitHomeUrl(): string {
  return resolveMcpBaseUrl()
}

/** Builds the slugless MCP URL shown in the standalone onboarding CLI snippet. */
export function buildCanonicalMcpEndpointUrl(): string {
  return `${buildCockpitHomeUrl()}${MCP_PATH}`
}

/** Builds the Claude CLI command that registers BrowserOS over HTTP MCP. */
export function buildCanonicalMcpCliCommand(): string {
  const url = buildCanonicalMcpEndpointUrl()
  return `claude mcp add ${BROWSEROS_MCP_SERVER_NAME} ${url} --transport http --scope user`
}
