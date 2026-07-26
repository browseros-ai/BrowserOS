/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Top-level interface every adapter runtime implements.
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

  getStatusSnapshot(): RuntimeStatusSnapshot
  subscribe(listener: StateListener): Unsubscribe
  getCapabilities(): ReadonlyArray<RuntimeCapability>

  executeAction(
    action: RuntimeAction,
    options?: { onLog?: (msg: string) => void },
  ): Promise<void>

  /** Build the shell-command string acpx-core spawns to run `spec`. */
  buildExecArgv(spec: ExecSpec): string

  /** Per-agent home dir on host. */
  getPerAgentHomeDir(agentId: string): string
}
