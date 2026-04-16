/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Centralized external service URLs.
 */

export const EXTERNAL_URLS = {
  KLAVIS_PROXY: 'https://llm.browseros.com/klavis',
  POSTHOG_DEFAULT: 'https://us.i.posthog.com',
  CODEGEN_SERVICE: 'https://graph.browseros.com',
  OPENAI_AUTH: 'https://auth.openai.com/oauth/authorize',
  OPENAI_TOKEN: 'https://auth.openai.com/oauth/token',
  SKILLS_CATALOG: 'https://cdn.browseros.com/skills/v1/catalog.json',
  GITHUB_DEVICE_CODE: 'https://github.com/login/device/code',
  GITHUB_OAUTH_TOKEN: 'https://github.com/login/oauth/access_token',
  GITHUB_COPILOT_API: 'https://api.githubcopilot.com',
  QWEN_DEVICE_CODE: 'https://chat.qwen.ai/api/v1/oauth2/device/code',
  QWEN_OAUTH_TOKEN: 'https://chat.qwen.ai/api/v1/oauth2/token',
  QWEN_CODE_API: 'https://portal.qwen.ai/v1',
  REFERRAL_SERVICE: 'https://browseros-referral.fly.dev',
} as const
