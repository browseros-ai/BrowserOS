/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Centralized file system paths.
 *
 * Retention and sizing values can be overridden via environment variables.
 */

import { configStore } from './config-store'

export const PATHS = {
  DEFAULT_EXECUTION_DIR: process.cwd(),
  BROWSEROS_DIR_NAME: '.browseros',
  DEV_BROWSEROS_DIR_NAME: '.browseros-dev',
  CACHE_DIR_NAME: 'cache',
  DB_DIR_NAME: 'db',
  DB_FILE_NAME: 'browseros.sqlite',
  MEMORY_DIR_NAME: 'memory',
  SESSIONS_DIR_NAME: 'sessions',
  TOOL_OUTPUT_DIR_NAME: 'tool-output',
  SOUL_FILE_NAME: 'SOUL.md',
  CORE_MEMORY_FILE_NAME: 'CORE.md',
  SKILLS_DIR_NAME: 'skills',
  BUILTIN_DIR_NAME: 'builtin',
  SERVER_CONFIG_FILE_NAME: 'server.json',
  OPENCLAW_DIR_NAME: 'openclaw',
  get SOUL_MAX_LINES() {
    return configStore.get('PATHS.SOUL_MAX_LINES')
  },
  get MEMORY_RETENTION_DAYS() {
    return configStore.get('PATHS.MEMORY_RETENTION_DAYS')
  },
  get SESSION_RETENTION_DAYS() {
    return configStore.get('PATHS.SESSION_RETENTION_DAYS')
  },
}
