/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Process-wide accessor for the live `BrowserSession` the cockpit
 * binds tool dispatches against. Today the cockpit module runs in
 * its own Bun process and the session stays null; tool dispatches
 * short-circuit with a structured "browser session not connected"
 * error so the wire shape is honest about the gap.
 *
 * The runtime merge step wires the real session at startup by
 * calling `setBrowserSession(...)` once apps/server has constructed
 * its CDP connection. Tests use the same setter to inject a stub.
 */

import type { BrowserSession } from '@browseros/server/browser/core/session'

let session: BrowserSession | null = null

export function getBrowserSession(): BrowserSession | null {
  return session
}

export function setBrowserSession(next: BrowserSession | null): void {
  session = next
}
