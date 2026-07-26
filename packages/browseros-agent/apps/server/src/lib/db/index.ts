/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */
import {
  type BrowserOsDatabase,
  type DbHandle,
  type OpenDbOptions,
  openBrowserOsDatabase,
} from './client'

let handle: DbHandle | null = null

/** Initializes the process-wide BrowserOS database handle used by server services. */
export function initializeDb(options: OpenDbOptions): DbHandle {
  if (!handle) {
    handle = openBrowserOsDatabase(options)
  }
  return handle
}

export function getDbHandle(): DbHandle {
  if (!handle) {
    throw new Error('Database not initialized. Call initializeDb() first.')
  }
  return handle
}

export function getDb(): BrowserOsDatabase {
  return getDbHandle().db
}

export function closeDb(): void {
  if (handle) {
    handle.sqlite.close()
    handle = null
  }
}

export type { BrowserOsDatabase, DbHandle, OpenDbOptions }
