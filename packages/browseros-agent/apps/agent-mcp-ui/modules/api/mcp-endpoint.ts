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
 * TODO(temporary cockpit mount): while `apps/agent-mcp-interface`
 * lives as a sub-route under `apps/server` (mounted at
 * `/cockpit` so it can borrow the server's live BrowserSession and
 * CDP attach), this builder targets the mounted shape
 * `http://127.0.0.1:9100/cockpit/mcp/<slug>`. When the BrowserOS
 * Chromium runtime ships direct CDP integration for the
 * agent-mcp-interface package, the interface will bind its own port
 * and `buildMcpEndpointUrl` will return
 * `http://127.0.0.1:<interface-port>/mcp/<slug>`. The unmount lives
 * in three commits: (1) drop the cockpit mount from
 * `apps/server/src/api/routes/index.ts`, (2) flip the constants
 * imported below, (3) remove the legacy `/cockpit/mcp/<slug>` arm
 * from `slugFromMcpEndpointUrl` once profile migration has run.
 */

import {
  COCKPIT_MOUNT_PREFIX,
  PROD_API_PORT,
} from '@browseros/agent-mcp-interface/shared/port'

/**
 * URL the UI shows in the copy widget and embeds in host-agent config
 * snippets. Matches the URL `apps/agent-mcp-interface` returns from
 * its `agents` service when running mounted, so a profile created
 * server-side and one composed client-side produce identical strings.
 */
export function buildMcpEndpointUrl(slug: string): string {
  return `http://127.0.0.1:${PROD_API_PORT}${COCKPIT_MOUNT_PREFIX}/mcp/${slug}`
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
