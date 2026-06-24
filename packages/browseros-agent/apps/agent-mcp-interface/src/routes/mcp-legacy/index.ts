/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Stub route that takes the place of the legacy per-slug MCP when
 * `COCKPIT_LEGACY_PER_AGENT_MCP` is `false` (the default). It returns
 * 404 with a structured body pointing at the v2 endpoint so any
 * harness with a stale URL fails loudly with a usable hint rather
 * than getting a generic Hono 404.
 *
 * The route module deliberately mirrors the real `mcpRoute` mount
 * shape so swapping between them in `server.ts` stays a one-line
 * conditional.
 */

import { Hono } from 'hono'

export const mcpDisabledRoute = new Hono().all('/mcp/:slug', (c) =>
  c.json(
    {
      error: 'legacy per-agent MCP endpoint is disabled',
      hint: 'use the standard endpoint at /cockpit/mcp, or set COCKPIT_LEGACY_PER_AGENT_MCP=1 to re-enable per-slug routes',
    },
    404,
  ),
)
