/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Centralized timeout configuration.
 *
 * All values can be overridden via environment variables.
 * Invalid values (non-numeric, negative) fall back to defaults.
 */

export const KLAVIS_PROXY_RETRY_BACKOFF_MS = [
  5_000, 10_000, 20_000, 40_000, 60_000,
] as const

/** Read a millisecond timeout from env, returning fallback on invalid input.
 * Only accepts pure integer strings (optional leading underscores stripped).
 * Rejects partial parses like "10s", "1.5", "abc". */
function envMs(name: string, fallback: number): number {
  const raw = process.env[name]
  if (raw === undefined) return fallback
  const normalized = raw.trim().replace(/_/g, '')
  if (!/^\d+$/.test(normalized)) return fallback
  const parsed = Number(normalized)
  if (!Number.isSafeInteger(parsed) || parsed < 0) return fallback
  return parsed
}

export const TIMEOUTS = {
  // Agent/Tool execution
  TOOL_CALL: envMs('BROWSEROS_TIMEOUT_TOOL_CALL', 120_000),
  TOOL_POST_ACTION: envMs('BROWSEROS_TIMEOUT_TOOL_POST_ACTION', 2_000),
  TEST_PROVIDER: envMs('BROWSEROS_TIMEOUT_TEST_PROVIDER', 15_000),
  REFINE_PROMPT: envMs('BROWSEROS_TIMEOUT_REFINE_PROMPT', 30_000),

  // MCP operations
  MCP_DEFAULT: envMs('BROWSEROS_TIMEOUT_MCP_DEFAULT', 5_000),
  MCP_TRANSPORT_PROBE: envMs('BROWSEROS_TIMEOUT_MCP_TRANSPORT_PROBE', 5_000),
  MCP_CLIENT_CONNECT: envMs('BROWSEROS_TIMEOUT_MCP_CLIENT_CONNECT', 15_000),

  // CDP connection
  CDP_CONNECT: envMs('BROWSEROS_TIMEOUT_CDP_CONNECT', 10_000),
  CDP_CONNECT_RETRY_DELAY: envMs('BROWSEROS_TIMEOUT_CDP_CONNECT_RETRY_DELAY', 1_000),
  CDP_RECONNECT_DELAY: envMs('BROWSEROS_TIMEOUT_CDP_RECONNECT_DELAY', 5_000),
  CDP_KEEPALIVE_INTERVAL: envMs('BROWSEROS_TIMEOUT_CDP_KEEPALIVE_INTERVAL', 30_000),
  CDP_KEEPALIVE_TIMEOUT: envMs('BROWSEROS_TIMEOUT_CDP_KEEPALIVE_TIMEOUT', 10_000),
  CDP_REQUEST_TIMEOUT: envMs('BROWSEROS_TIMEOUT_CDP_REQUEST_TIMEOUT', 60_000),

  // External API calls
  KLAVIS_FETCH: envMs('BROWSEROS_TIMEOUT_KLAVIS_FETCH', 30_000),
  SKILLS_FETCH: envMs('BROWSEROS_TIMEOUT_SKILLS_FETCH', 15_000),
  SKILLS_SYNC_INTERVAL: envMs('BROWSEROS_TIMEOUT_SKILLS_SYNC_INTERVAL', 45 * 60_000),

  // Navigation/DOM
  NAVIGATION: envMs('BROWSEROS_TIMEOUT_NAVIGATION', 10_000),
  PAGE_LOAD_WAIT: envMs('BROWSEROS_TIMEOUT_PAGE_LOAD_WAIT', 30_000),
  PAGE_LOAD_POLL_INTERVAL: envMs('BROWSEROS_TIMEOUT_PAGE_LOAD_POLL_INTERVAL', 150),
  STABLE_DOM: envMs('BROWSEROS_TIMEOUT_STABLE_DOM', 3_000),
  FILE_CHOOSER: envMs('BROWSEROS_TIMEOUT_FILE_CHOOSER', 3_000),

  // OAuth
  OAUTH_FLOW_TTL: envMs('BROWSEROS_TIMEOUT_OAUTH_FLOW_TTL', 300_000),
  OAUTH_TOKEN_EXPIRY_BUFFER: envMs('BROWSEROS_TIMEOUT_OAUTH_TOKEN_EXPIRY_BUFFER', 300_000),
  OAUTH_POLL_INTERVAL: envMs('BROWSEROS_TIMEOUT_OAUTH_POLL_INTERVAL', 2_000),
  OAUTH_POLL_TIMEOUT: envMs('BROWSEROS_TIMEOUT_OAUTH_POLL_TIMEOUT', 300_000),
  DEVICE_CODE_POLL_SAFETY_MARGIN: envMs('BROWSEROS_TIMEOUT_DEVICE_CODE_POLL_SAFETY_MARGIN', 3_000),
}

export type TimeoutKey = keyof typeof TIMEOUTS
