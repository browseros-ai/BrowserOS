/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Centralized file system paths.
 */

export const PATHS = {
  DEFAULT_EXECUTION_DIR: process.cwd(),
  BROWSEROS_DIR_NAME: '.browseros',
  DEV_BROWSEROS_DIR_NAME: '.browseros-dev',
  BROWSERCLAW_DIR_NAME: '.browserclaw',
  DEV_BROWSERCLAW_DIR_NAME: '.browserclaw-dev',
  CACHE_DIR_NAME: 'cache',
  DB_DIR_NAME: 'db',
  DB_FILE_NAME: 'browseros.sqlite',
  SESSIONS_DIR_NAME: 'sessions',
  TOOL_OUTPUT_DIR_NAME: 'tool-output',
  SOUL_FILE_NAME: 'SOUL.md',
  SERVER_CONFIG_FILE_NAME: 'server.json',
  SESSION_RETENTION_DAYS: 30,
} as const
