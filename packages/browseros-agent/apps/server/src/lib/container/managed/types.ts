/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Types shared by the abstract `ManagedContainer` base and its
 * subclasses. Kept in their own file so the base class file can stay
 * focused on behaviour.
 */

/** Subset of `NodeJS.Platform` — kept loose so a future Linux add
 *  doesn't require touching the abstraction. */
export type Platform = NodeJS.Platform

/** State the container is in, viewed from the harness's perspective. */
export type ContainerState =
  | 'not_installed'
  | 'installing'
  | 'installed'
  | 'starting'
  | 'running'
  | 'stopped'
  | 'errored'

/** Subset of the state machine that callers should treat as "may
 *  resolve to running on its own with time" — `execProcess` and
 *  friends wait through these. Callers checking `isReady()` use this. */
export const TRANSIENT_STATES: ReadonlySet<ContainerState> = new Set([
  'installing',
  'starting',
])

export interface ContainerDescriptor {
  /** Stable id matching `agent.adapter` for harness lookups. */
  adapterId: string
  /** Human-readable label for UI. */
  displayName: string
  /** Image ref to pull on `install()`. */
  defaultImage: string
  /** Fixed container name inside the VM. */
  containerName: string
  /** Platforms where this container is supported (today: ['darwin']). */
  platforms: ReadonlyArray<Platform>
  /** Optional probe tuning. */
  readinessProbe?: {
    timeoutMs?: number
    intervalMs?: number
  }
}

/**
 * Mount root the container exposes. `kind` is informational —
 * `'shared'` mounts are bind-mounted once and used by every agent;
 * `'per-agent'` mounts are bind-mounted per agent and the harness's
 * agent-delete flow is expected to clean them up. Both are
 * containment-checked by `toContainerPath` / `toHostPath`.
 */
export interface MountRoot {
  hostPath: string
  containerPath: string
  kind: 'shared' | 'per-agent'
}

export interface ContainerStatusSnapshot {
  adapterId: string
  containerName: string
  state: ContainerState
  lastError: string | null
  lastErrorAt: number | null
}

/** What the caller wants to run inside the container. */
export interface ExecSpec {
  /** Argv inside the container — first element is the executable. */
  argv: readonly [string, ...string[]]
  env?: Record<string, string>
}

export interface ExecResult {
  exitCode: number
  stdout: string
  stderr: string
}

/** Reset granularity — see the architecture doc / team brief. */
export type ResetLevel = 'soft' | 'wipe-agent' | 'hard'

export interface ResetOptions {
  /** Required for `level === 'wipe-agent'` — agent id whose home dir
   *  should be removed. Ignored for soft/hard. */
  agentId?: string
  onLog?: (msg: string) => void
}
