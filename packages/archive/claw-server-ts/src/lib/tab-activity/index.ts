/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Process-wide singleton registry. Bound to the same
 * `getBrowserSession` accessor the rest of the cockpit uses so the
 * registry sees the same `PageManager` instance the tool dispatches
 * write to.
 */

import { getBrowserSession } from '../browser-session'
import { createTabActivityRegistry, type TabActivityRegistry } from './registry'

export const tabActivityRegistry: TabActivityRegistry =
  createTabActivityRegistry({ getSession: getBrowserSession })

export { extractPageId, TOOLS_WITH_PAGE } from './extract-page-id'
export type { TabActivityRecord, TabActivityRegistry } from './registry'
