/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Top-level interface every adapter runtime implements. Two abstract
 * subclasses (`ContainerAgentRuntime`, `HostProcessAgentRuntime`)
 * cover the two kinds we ship today.
 */

import type {
  ExecSpec,
  RuntimeAction,
  RuntimeCapability,
  RuntimeDescriptor,
  RuntimeStatusSnapshot,
  StateListener,
  Unsubscribe,
} from './types'

export interface AgentRuntime {
  readonly descriptor: RuntimeDescriptor

  // ── Status surface (Plane B feed) ────────────────────────────────
  getStatusSnapshot(): RuntimeStatusSnapshot
  subscribe(listener: StateListener): Unsubscribe
  getCapabilities(): ReadonlyArray<RuntimeCapability>

  // ── Action dispatch (Plane B control) ────────────────────────────
  executeAction(
    action: RuntimeAction,
    options?: { onLog?: (msg: string) => void },
  ): Promise<void>

  // ── ACP plane integration ────────────────────────────────────────
  /**
   * Build the shell-command string acpx-core spawns to run `spec`
   * against this runtime. For container kinds, this is the
   * `limactl shell <vm> -- nerdctl exec -i …` chain; for host kinds,
   * it's `env KEY=VAL <binary> <argv...>`.
   */
  buildExecArgv(spec: ExecSpec): string

  // ── Filesystem ───────────────────────────────────────────────────
  /** Per-agent home dir on host. Both kinds expose this; container
   *  kinds also expose `toContainerPath` for in-container translation. */
  getPerAgentHomeDir(agentId: string): string
  toContainerPath?(hostPath: string): string
  toHostPath?(containerPath: string): string
}
