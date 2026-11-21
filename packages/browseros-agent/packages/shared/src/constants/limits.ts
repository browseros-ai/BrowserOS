/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Centralized limits and thresholds.
 */

export const RECORDING_INGEST_MAX_BYTES = 16 * 1024 * 1024
/** Aggregate target used until a server explicitly advertises a larger ceiling. */
export const RECORDING_INGEST_FALLBACK_MAX_BYTES = 2 * 1024 * 1024
