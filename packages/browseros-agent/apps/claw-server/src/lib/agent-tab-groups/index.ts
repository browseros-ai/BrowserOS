/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { createTabGroupTracker } from './tracker'

export type { TabGroupColor } from './group-color'
export {
  colorForSlug,
  hexForSlug,
  TAB_GROUP_COLORS,
  TAB_GROUP_HEX,
} from './group-color'
export type {
  RecordOpenInput,
  RememberGroupInput,
  TabGroupRecord,
  TabGroupTracker,
} from './tracker'
export { createTabGroupTracker } from './tracker'

/** Process-wide singleton consumed by the v2 dispatch path and the focus route. */
export const tabGroupTracker = createTabGroupTracker()
