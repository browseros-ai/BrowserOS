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

import { configStore } from './config-store'

export const KLAVIS_PROXY_RETRY_BACKOFF_MS = [
  5_000, 10_000, 20_000, 40_000, 60_000,
] as const

export const TIMEOUTS = {
  // Agent/Tool execution
  get TOOL_CALL() {
    return configStore.get('TIMEOUTS.TOOL_CALL')
  },
  get TOOL_POST_ACTION() {
    return configStore.get('TIMEOUTS.TOOL_POST_ACTION')
  },
  get TEST_PROVIDER() {
    return configStore.get('TIMEOUTS.TEST_PROVIDER')
  },
  get REFINE_PROMPT() {
    return configStore.get('TIMEOUTS.REFINE_PROMPT')
  },

  // MCP operations
  get MCP_DEFAULT() {
    return configStore.get('TIMEOUTS.MCP_DEFAULT')
  },
  get MCP_TRANSPORT_PROBE() {
    return configStore.get('TIMEOUTS.MCP_TRANSPORT_PROBE')
  },
  get MCP_CLIENT_CONNECT() {
    return configStore.get('TIMEOUTS.MCP_CLIENT_CONNECT')
  },

  // CDP connection
  get CDP_CONNECT() {
    return configStore.get('TIMEOUTS.CDP_CONNECT')
  },
  get CDP_CONNECT_RETRY_DELAY() {
    return configStore.get('TIMEOUTS.CDP_CONNECT_RETRY_DELAY')
  },
  get CDP_RECONNECT_DELAY() {
    return configStore.get('TIMEOUTS.CDP_RECONNECT_DELAY')
  },
  get CDP_KEEPALIVE_INTERVAL() {
    return configStore.get('TIMEOUTS.CDP_KEEPALIVE_INTERVAL')
  },
  get CDP_KEEPALIVE_TIMEOUT() {
    return configStore.get('TIMEOUTS.CDP_KEEPALIVE_TIMEOUT')
  },
  get CDP_REQUEST_TIMEOUT() {
    return configStore.get('TIMEOUTS.CDP_REQUEST_TIMEOUT')
  },

  // External API calls
  get KLAVIS_FETCH() {
    return configStore.get('TIMEOUTS.KLAVIS_FETCH')
  },
  get SKILLS_FETCH() {
    return configStore.get('TIMEOUTS.SKILLS_FETCH')
  },
  get SKILLS_SYNC_INTERVAL() {
    return configStore.get('TIMEOUTS.SKILLS_SYNC_INTERVAL')
  },

  // Navigation/DOM
  get NAVIGATION() {
    return configStore.get('TIMEOUTS.NAVIGATION')
  },
  get PAGE_LOAD_WAIT() {
    return configStore.get('TIMEOUTS.PAGE_LOAD_WAIT')
  },
  get PAGE_LOAD_POLL_INTERVAL() {
    return configStore.get('TIMEOUTS.PAGE_LOAD_POLL_INTERVAL')
  },
  get STABLE_DOM() {
    return configStore.get('TIMEOUTS.STABLE_DOM')
  },
  get FILE_CHOOSER() {
    return configStore.get('TIMEOUTS.FILE_CHOOSER')
  },

  // OAuth
  get OAUTH_FLOW_TTL() {
    return configStore.get('TIMEOUTS.OAUTH_FLOW_TTL')
  },
  get OAUTH_TOKEN_EXPIRY_BUFFER() {
    return configStore.get('TIMEOUTS.OAUTH_TOKEN_EXPIRY_BUFFER')
  },
  get OAUTH_POLL_INTERVAL() {
    return configStore.get('TIMEOUTS.OAUTH_POLL_INTERVAL')
  },
  get OAUTH_POLL_TIMEOUT() {
    return configStore.get('TIMEOUTS.OAUTH_POLL_TIMEOUT')
  },
  get DEVICE_CODE_POLL_SAFETY_MARGIN() {
    return configStore.get('TIMEOUTS.DEVICE_CODE_POLL_SAFETY_MARGIN')
  },
}

export type TimeoutKey = keyof typeof TIMEOUTS
