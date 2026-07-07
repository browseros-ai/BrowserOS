/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Singleton accessor for the `agent-mcp-manager` bound API. Manifest
 * lives at `<browserclawDir>/mcp-manager` so the per-cockpit-agent
 * server entries stay isolated from the BrowserOS-wide entry
 * `apps/server` manages under `<browserosDir>/mcp-manager`.
 *
 * Since 0.0.4 the library exposes a functional surface. `bind()`
 * pre-fills the workspaceDir on every verb so call sites stay
 * `mgr.link({...})`, `mgr.rescan()`, etc. Scope is always 'system'
 * here since cockpit agents are user-wide; per-call `scope` overrides
 * are still available via the input object.
 */

import { join } from 'node:path'
import { type BoundApi, bind } from 'agent-mcp-manager'
import { getClawServerDir } from './browserclaw-dir'

let cached: BoundApi | null = null

export function getMcpManagerWorkspaceDir(): string {
  return join(getClawServerDir(), 'mcp-manager')
}

export function getMcpManager(): BoundApi {
  if (!cached) cached = bind(getMcpManagerWorkspaceDir())
  return cached
}

export function resetMcpManagerForTesting(): void {
  cached = null
}

export function setMcpManagerForTesting(stub: BoundApi): void {
  cached = stub
}
