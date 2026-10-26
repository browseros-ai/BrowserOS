/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

/** Width of the persistent sidebar on desktop, in px. */
export const SIDEBAR_WIDTH = 232

/** Every page shares this maximum width; there is no narrow variant. */
export const PAGE_MAX_WIDTH = 1040

/** The page enter transition, matching the design prototype. */
export const PAGE_ENTER_DURATION_S = 0.22
export const PAGE_ENTER_OFFSET_PX = 8
export const PAGE_ENTER_EASE = [0.2, 0.7, 0.3, 1] as const
