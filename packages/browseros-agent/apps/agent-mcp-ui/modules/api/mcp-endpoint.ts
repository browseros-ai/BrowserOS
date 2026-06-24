/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Canonical source for the URL the UI advertises as the MCP endpoint
 * for an agent, the CLI snippet shown alongside it, and the slug
 * parser used to read URLs that the server-rendered profile responds
 * with. Every "copy URL" widget and every "add to host agent" config
 * the wizard / directory pages render flows through these helpers,
 * so the future cutover is confined to this file.
 *
 * While `apps/agent-mcp-interface` lives as a sub-route under
 * `apps/server`, this builder targets the mounted `/cockpit/mcp/<slug>`
 * shape. It defaults to the production port but honors dev-launcher
 * overrides when `dev:agent-mcp:watch:new` selects a random port.
 */

import {
  BROWSEROS_MCP_SERVER_NAME,
  MCP_PATH,
} from '@browseros/agent-mcp-interface/shared/mcp-url'
import {
  COCKPIT_MOUNT_PREFIX,
  PROD_API_PORT,
} from '@browseros/agent-mcp-interface/shared/port'
import {
  API_URL_STORAGE_KEY,
  isLoopbackCockpitUrl,
  resolveApiBaseUrlFromSources,
} from './client.helpers'

function fallbackBaseUrl(): string {
  return `http://127.0.0.1:${PROD_API_PORT}${COCKPIT_MOUNT_PREFIX}`
}

/** Resolves the same cockpit base URL as the API client for pre-create previews. */
function resolveMcpBaseUrl(): string {
  const fallback = fallbackBaseUrl()
  if (typeof window === 'undefined') return fallback

  const query = new URLSearchParams(window.location.search).get('apiUrl')
  if (isLoopbackCockpitUrl(query)) {
    try {
      window.sessionStorage.setItem(API_URL_STORAGE_KEY, query)
    } catch {
      // sessionStorage may reject writes in sandboxed contexts; this call can still use the query URL.
    }
    return query
  }

  try {
    return resolveApiBaseUrlFromSources({
      query: null,
      stored: window.sessionStorage.getItem(API_URL_STORAGE_KEY),
      launcher: import.meta.env.VITE_BROWSEROS_AGENT_MCP_API_URL,
      fallback,
    })
  } catch {
    return resolveApiBaseUrlFromSources({
      query: null,
      stored: null,
      launcher: import.meta.env.VITE_BROWSEROS_AGENT_MCP_API_URL,
      fallback,
    })
  }
}

/**
 * URL the UI shows in the copy widget and embeds in host-agent config
 * snippets. Matches the URL `apps/agent-mcp-interface` returns from
 * its `agents` service when running mounted, so a profile created
 * server-side and one composed client-side produce identical strings.
 */
export function buildMcpEndpointUrl(slug: string): string {
  return `${resolveMcpBaseUrl()}/mcp/${slug}`
}

/**
 * Pulls the slug segment out of an MCP URL. Tolerates both the
 * mounted shape (`/cockpit/mcp/<slug>`) and the future direct shape
 * (`/mcp/<slug>`). Returns an empty string when neither matches so
 * callers can fall back to a known id.
 */
export function slugFromMcpEndpointUrl(url: string): string {
  const match = url.match(/\/mcp\/([^/?#]+)/)
  return match?.[1] ?? ''
}

/**
 * CLI snippet shown next to the URL widgets and copied as the
 * "add to host agent" command. Lives here so the directory and the
 * wizard render identical text from a single source.
 */
export function buildMcpCliCommand(slug: string): string {
  return `mcp add ${slug}`
}

/**
 * Canonical v2 URL the MCP page advertises: one slugless endpoint
 * for the whole cockpit. Uses the same base resolution as
 * `buildMcpEndpointUrl` so dev-launcher overrides and query-string
 * apiUrl forwarding stay consistent across both shapes.
 */
export function buildCanonicalMcpEndpointUrl(): string {
  return `${resolveMcpBaseUrl()}${MCP_PATH}`
}

/**
 * Canonical CLI snippet for one-click harnesses that ship their own
 * MCP CLI. Anthropic's `claude` CLI is the lead consumer; other
 * harnesses get the "Connect" button on the MCP page instead.
 */
export function buildCanonicalMcpCliCommand(): string {
  const url = buildCanonicalMcpEndpointUrl()
  return `claude mcp add ${BROWSEROS_MCP_SERVER_NAME} ${url} --transport http --scope user`
}
