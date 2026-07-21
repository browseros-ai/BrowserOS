/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Canonical source for the URL the UI advertises as the MCP endpoint
 * and the CLI snippets shown alongside it. Every "copy URL" widget
 * and every "add to host agent" config the wizard / directory pages
 * render flows through these helpers.
 *
 * BrowserOS-managed builds read the MCP proxy pref; dev and
 * standalone builds keep the existing launcher/fallback resolution.
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

/** Resolves the MCP proxy base URL from BrowserOS prefs or trusted fallbacks. */
export async function resolveMcpBaseUrl(): Promise<string> {
  return resolveBrowserOSMcpBaseUrl(apiBaseUrlSourcesFromWindow())
}

function mcpBaseUrlFallback(): string {
  return resolveApiBaseUrlFromSources(apiBaseUrlSourcesFromWindow())
}

/**
 * CLI snippet shown next to the URL widgets and copied as the
 * "add to host agent" command. The slug is the host-visible server
 * name, not part of the endpoint URL.
 */
export function buildMcpCliCommand(slug: string): string {
  return `mcp add ${slug}`
}

/**
 * Canonical v2 URL the MCP page advertises: one slugless endpoint
 * for the whole cockpit.
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
