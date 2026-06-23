/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

/**
 * Thrown when `executeAction` is called with an action that the
 * runtime's `getCapabilities()` doesn't list. The HTTP route layer
 * maps this to 405; the UI gates affordances on capabilities so a
 * well-behaved client should never trip this.
 */
export class ActionNotSupportedError extends Error {
  constructor(
    public readonly adapterId: string,
    public readonly actionType: string,
    public readonly capabilities: ReadonlyArray<string>,
  ) {
    super(
      `Runtime "${adapterId}" does not support action "${actionType}" ` +
        `(capabilities: ${capabilities.join(', ') || 'none'})`,
    )
    this.name = 'ActionNotSupportedError'
  }
}
