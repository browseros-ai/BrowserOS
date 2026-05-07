/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

export class VmError extends Error {
  constructor(message: string) {
    super(message)
    this.name = new.target.name
  }
}

export class VmNotReadyError extends VmError {}

export class VmStateCorruptedError extends VmError {}

export class LimaCommandError extends VmError {
  constructor(
    command: string,
    public readonly exitCode: number,
    public readonly stderr: string,
  ) {
    super(`${command} failed with exit code ${exitCode}: ${stderr}`)
  }
}

export class ContainerCliError extends VmError {
  constructor(
    command: string,
    public readonly exitCode: number,
    public readonly stderr: string,
    message = `${command} failed with exit code ${exitCode}: ${stderr}`,
  ) {
    super(message)
  }
}

export class ContainerNameInUseError extends ContainerCliError {
  constructor(
    public readonly containerName: string,
    command: string,
    exitCode: number,
    stderr: string,
  ) {
    super(
      command,
      exitCode,
      stderr,
      `${command} failed because container name "${containerName}" is already in use: ${stderr}`,
    )
  }
}

export class ContainerNameReleaseTimeoutError extends VmError {
  constructor(
    public readonly containerName: string,
    public readonly timeoutMs: number,
  ) {
    super(
      `Timed out waiting ${timeoutMs}ms for container name "${containerName}" to be released`,
    )
  }
}

/**
 * Container's process never reached `running` state within the
 * timeout window. Distinct from `ContainerNameReleaseTimeoutError`
 * (which is about deletion). Thrown by
 * `ContainerCli.waitForContainerRunning` and surfaced by the
 * managed-container layer when a `start()` finishes the create+start
 * commands but the container hasn't actually come up.
 */
export class ContainerNotRunningError extends VmError {
  constructor(
    public readonly containerName: string,
    public readonly timeoutMs: number,
  ) {
    super(
      `Timed out waiting ${timeoutMs}ms for container "${containerName}" to reach running state`,
    )
  }
}

export class ImageLoadError extends VmError {
  constructor(
    public readonly imageRef: string,
    message: string,
    public override readonly cause?: unknown,
  ) {
    super(`failed to load image ${imageRef}: ${message}`)
  }
}
