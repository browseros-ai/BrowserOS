/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Centralized URL configuration.
 */

import { CLAW_API_PORT_DEFAULT } from './ports'

export const MCP_PATH = '/mcp'
export const BROWSEROS_MCP_SERVER_NAME = 'BrowserClaw'

export function canonicalMcpUrlForPort(port = CLAW_API_PORT_DEFAULT): string {
  return `http://127.0.0.1:${port}${MCP_PATH}`
}

export const EXTERNAL_URLS = {
  CDN: 'https://cdn.browseros.com',
  KLAVIS_PROXY: 'https://llm.browseros.com/klavis',
  POSTHOG_DEFAULT: 'https://us.i.posthog.com',
  OPENAI_AUTH: 'https://auth.openai.com/oauth/authorize',
  OPENAI_TOKEN: 'https://auth.openai.com/oauth/token',
  GITHUB_DEVICE_CODE: 'https://github.com/login/device/code',
  GITHUB_OAUTH_TOKEN: 'https://github.com/login/oauth/access_token',
  GITHUB_COPILOT_API: 'https://api.githubcopilot.com',
  QWEN_DEVICE_CODE: 'https://chat.qwen.ai/api/v1/oauth2/device/code',
  QWEN_OAUTH_TOKEN: 'https://chat.qwen.ai/api/v1/oauth2/token',
  QWEN_CODE_API: 'https://portal.qwen.ai/v1',
} as const
