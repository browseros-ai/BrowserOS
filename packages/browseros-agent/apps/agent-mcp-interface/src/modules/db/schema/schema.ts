/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Barrel re-export for the audit-log SQLite schema. drizzle-kit and
 * the runtime drizzle client both read this file as the schema entry
 * point. One file per table, re-exported here.
 */

export * from './tool-dispatches.sql'
