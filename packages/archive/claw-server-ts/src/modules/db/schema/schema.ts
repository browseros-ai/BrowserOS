/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Barrel re-export for the audit-log SQLite schema. drizzle-kit and
 * the runtime drizzle client both read this file as the schema entry
 * point. One file per table, re-exported here.
 */

export * from './agent-session-ends.sql'
export * from './agent-session-starts.sql'
export * from './recording-batches.sql'
export * from './recording-payloads.sql'
export * from './recording-streams.sql'
export * from './session-tabs.sql'
export * from './tab-claims.sql'
export * from './tab-recordings.sql'
export * from './tool-dispatches.sql'
