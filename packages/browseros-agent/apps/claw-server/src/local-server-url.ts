/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Module-local singleton holding the URL the Hono server bound to.
 * Set once by main.ts after Bun.serve resolves; read by anything that
 * needs to compose URLs reachable from inside the same process (e.g.
 * the future per-agent MCP route mux will embed this in mcpServers
 * configs handed to spawned host agents).
 *
 * Plain mutable string rather than a hook/store because writer and
 * reader live in the same Bun process and the value is written
 * exactly once at boot.
 */

let localServerUrl: string | null = null

export function setLocalServerUrl(url: string): void {
  localServerUrl = url
}

/**
 * Returns null when the server has not bound yet. Callers that
 * depend on the URL should treat null as "not ready"; the boot path
 * always sets it before any route handler executes.
 */
export function getLocalServerUrl(): string | null {
  return localServerUrl
}
