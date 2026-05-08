/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Shared types for the AgentRuntime layer. Pure types — no behaviour
 * lives here.
 */

import type { ExecSpec } from '../../container/managed'

export type Platform = NodeJS.Platform

export interface RuntimeDescriptor {
  /** Stable id matching `agent.adapter` for harness lookups. */
  adapterId: string
  /** Human-readable label for UI. */
  displayName: string
  /** Discriminator for runtime kind. UI components route on this. */
  kind: 'container' | 'host-process'
  /** Platforms where this runtime is supported (today: ['darwin']
   *  for container kinds; varies for host kinds). */
  platforms: ReadonlyArray<Platform>
}

export type RuntimeState =
  | 'unsupported_platform'
  | 'errored'
  // container-only
  | 'not_installed'
  | 'installing'
  | 'installed'
  | 'starting'
  | 'running'
  | 'stopped'
  // host-only
  | 'cli_missing'
  | 'cli_present'
  | 'cli_unhealthy'

export interface RuntimeStatusSnapshot {
  adapterId: string
  state: RuntimeState
  /** True iff the harness can spawn turns against this runtime now. */
  isReady: boolean
  lastError: string | null
  lastErrorAt: number | null
  /** Wall-clock ms when the last definitive readiness probe completed.
   *  Null when the runtime has never been probed. Distinct from
   *  `lastErrorAt` (only set on errors) so consumers can read probe
   *  staleness regardless of health state. */
  probedAt?: number | null
  /** Adapter-specific structured fields the UI may render. Keep keys
   *  stable so the UI can opt into them. */
  details?: Record<string, unknown>
}

export type RuntimeCapability =
  | 'install'
  | 'start'
  | 'stop'
  | 'restart'
  | 'reset-soft'
  | 'reset-wipe-agent'
  | 'reset-hard'
  | 'logs'
  | 'terminal'
  | 'reinstall-cli'
  | 'check-auth'
  | 'gateway-control-plane'
  | 'agent-crud-via-runtime'

/**
 * Discriminated union of every action a runtime can be asked to
 * perform. Required arguments live on the variant so callers can't
 * forget them (e.g. `agentId` for `reset-wipe-agent`).
 */
export type RuntimeAction =
  | { type: 'install' }
  | { type: 'start' }
  | { type: 'stop' }
  | { type: 'restart' }
  | { type: 'reset-soft' }
  | { type: 'reset-wipe-agent'; agentId: string }
  | { type: 'reset-hard' }
  | { type: 'reinstall-cli' }
  | { type: 'check-auth' }

export type StateListener = (snapshot: RuntimeStatusSnapshot) => void
export type Unsubscribe = () => void

export type { ExecSpec }
