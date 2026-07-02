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
 * BrowserOS-managed builds read the MCP proxy pref; dev and
 * standalone builds keep the existing launcher/fallback resolution.
 */

import {
  BROWSEROS_MCP_SERVER_NAME,
  MCP_PATH,
} from '@browseros/claw-server/shared/mcp-url'
import {
  apiBaseUrlSourcesFromWindow,
  resolveBrowserOSMcpBaseUrl,
} from './browseros-ports'
import { resolveApiBaseUrlFromSources } from './client.helpers'

/** Resolves the MCP proxy base URL from BrowserOS prefs or trusted fallbacks. */
export async function resolveMcpBaseUrl(): Promise<string> {
  return resolveBrowserOSMcpBaseUrl(apiBaseUrlSourcesFromWindow())
}

function mcpBaseUrlFallback(): string {
  return resolveApiBaseUrlFromSources(apiBaseUrlSourcesFromWindow())
}

/**
 * URL the UI shows in the copy widget and embeds in host-agent config
 * snippets. Matches the URL `apps/claw-server` returns from its
 * `agents` service so server-created and client-composed profiles
 * produce identical strings.
 */
export function buildMcpEndpointUrl(slug: string): string {
  return `${mcpBaseUrlFallback()}/mcp/${slug}`
}

/** Builds a per-agent MCP endpoint after BrowserOS proxy-port resolution. */
export async function resolveMcpEndpointUrl(slug: string): Promise<string> {
  return `${await resolveMcpBaseUrl()}/mcp/${slug}`
}

/**
 * Pulls the slug segment out of an MCP URL. Tolerates both the
 * removed prefixed shape and the current direct shape. Returns an
 * empty string when neither matches so callers can fall back to a
 * known id.
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
  return `${mcpBaseUrlFallback()}${MCP_PATH}`
}

/** Builds the canonical MCP endpoint after BrowserOS proxy-port resolution. */
export async function resolveCanonicalMcpEndpointUrl(): Promise<string> {
  return `${await resolveMcpBaseUrl()}${MCP_PATH}`
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
