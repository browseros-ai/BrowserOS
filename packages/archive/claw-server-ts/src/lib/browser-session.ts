/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Process-wide accessor for the live BrowserSession that tool
 * dispatches use after the standalone server attaches to BrowserOS.
 */

import type { BrowserSession } from '@browseros/browser-core/core/session'

let session: BrowserSession | null = null

export function getBrowserSession(): BrowserSession | null {
  return session
}

export function setBrowserSession(next: BrowserSession | null): void {
  session = next
}
