/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Singleton accessor for the active `BrowserExecutor` provider.
 * Today the only provider is the stub; a future Chromium provider
 * picks itself based on `env.executorProvider` without churning
 * callers in `src/mcp/`.
 */

import { StubBrowserExecutor } from './stub'
import type { BrowserExecutor } from './types'

export { StubBrowserExecutor } from './stub'
export * from './types'

let cached: BrowserExecutor | null = null

export function getExecutor(): BrowserExecutor {
  if (!cached) cached = new StubBrowserExecutor()
  return cached
}

/** Test seam: swap the active provider. Restore by calling with null. */
export function setExecutorForTesting(stub: BrowserExecutor | null): void {
  cached = stub
}
