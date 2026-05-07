/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { ContainerState } from './types'

/**
 * Thrown by `ManagedContainer.execProcess` / `execOneShot` when the
 * container isn't in `running` state and can't be made ready in time
 * (or shouldn't be — `errored` and `not_installed` are terminal until
 * the user takes action). The harness route layer catches this and
 * surfaces a structured error frame on the chat SSE so the UI can
 * render an actionable banner instead of a silent broken turn.
 *
 * `reason` discriminates so the UI can pick the right next-action:
 * - `not_installed` → "Set up containers"
 * - `installing` / `starting` → "Container is starting, please wait"
 * - `stopped` → "Start container"
 * - `errored` → "Reset container"
 * - `timeout` → "Container is taking too long, try again"
 */
export class ContainerNotReadyError extends Error {
  constructor(
    public readonly state: ContainerState,
    public readonly containerId: string,
    public readonly reason:
      | 'not_installed'
      | 'installing'
      | 'starting'
      | 'stopped'
      | 'errored'
      | 'timeout',
    public readonly hint: string,
    public readonly lastError?: string | null,
  ) {
    super(
      `Container "${containerId}" is not ready (state=${state}, reason=${reason}): ${hint}` +
        (lastError ? ` — last error: ${lastError}` : ''),
    )
    this.name = 'ContainerNotReadyError'
  }
}

/** Thrown when a `reset(level)` arrives with a level that the
 *  current `ManagedContainer` implementation doesn't support yet
 *  (e.g. `'hard'` requires a `VmService` that hasn't shipped). */
export class ResetNotSupportedError extends Error {
  constructor(
    public readonly level: string,
    message = `reset(${level}) is not supported in this build`,
  ) {
    super(message)
    this.name = 'ResetNotSupportedError'
  }
}

/** Thrown by `toContainerPath` / `toHostPath` when the input path
 *  doesn't sit under any declared mount root. */
export class PathOutsideMountsError extends Error {
  constructor(
    public readonly path: string,
    public readonly direction: 'host->container' | 'container->host',
  ) {
    super(
      `Path "${path}" is not inside any declared mount root for direction ${direction}`,
    )
    this.name = 'PathOutsideMountsError'
  }
}
