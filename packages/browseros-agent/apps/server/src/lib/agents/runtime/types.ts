/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Shared types for the AgentRuntime layer. Pure types - no behaviour
 * lives here.
 */

export type Platform = NodeJS.Platform

export interface ExecSpec {
  argv: string[]
  env?: Record<string, string>
}

export interface RuntimeDescriptor {
  adapterId: string
  displayName: string
  kind: 'host-process'
  platforms: ReadonlyArray<Platform>
}

export type RuntimeState =
  | 'unsupported_platform'
  | 'errored'
  | 'cli_missing'
  | 'cli_present'
  | 'cli_unhealthy'

export interface RuntimeStatusSnapshot {
  adapterId: string
  state: RuntimeState
  isReady: boolean
  lastError: string | null
  lastErrorAt: number | null
  probedAt?: number | null
  details?: Record<string, unknown>
}

export type RuntimeCapability = 'reinstall-cli' | 'check-auth'

export type RuntimeAction = { type: 'reinstall-cli' } | { type: 'check-auth' }

export type StateListener = (snapshot: RuntimeStatusSnapshot) => void
export type Unsubscribe = () => void
