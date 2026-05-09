/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Centralized file system paths.
 *
 * Retention and sizing values can be overridden via environment variables.
 */

/** Read a positive integer limit from env, returning fallback on invalid input.
 * Only accepts pure integer strings (optional leading underscores stripped).
 * Rejects partial parses like "30s", "1.5", "abc". */
function envInt(name: string, fallback: number): number {
  const raw = process.env[name]
  if (raw === undefined) return fallback
  const normalized = raw.trim().replace(/_/g, '')
  if (!/^\d+$/.test(normalized)) return fallback
  const parsed = Number(normalized)
  if (!Number.isSafeInteger(parsed) || parsed < 0) return fallback
  return parsed
}

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
  SOUL_MAX_LINES: envInt('BROWSEROS_LIMIT_SOUL_MAX_LINES', 150),
  MEMORY_RETENTION_DAYS: envInt('BROWSEROS_LIMIT_MEMORY_RETENTION_DAYS', 30),
  SESSION_RETENTION_DAYS: envInt('BROWSEROS_LIMIT_SESSION_RETENTION_DAYS', 30),
}
