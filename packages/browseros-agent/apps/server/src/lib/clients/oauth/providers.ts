/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { EXTERNAL_URLS } from '@browseros/shared/constants/urls'

export interface OAuthProviderConfig {
  id: string
  name: string
  clientId: string
  authEndpoint: string
  tokenEndpoint: string
  scopes: string[]
  extraAuthParams?: Record<string, string>
  upstreamLLMProvider: string
  authFlow?: 'pkce' | 'device-code'
  /** Device code flow uses form-urlencoded instead of JSON */
  deviceCodeContentType?: 'json' | 'form'
  /** Device code flow requires PKCE code_challenge/code_verifier */
  deviceCodeRequiresPKCE?: boolean
}

export const OAUTH_PROVIDERS: Record<string, OAuthProviderConfig> = {
  // Google OAuth - ล็อกอินด้วย Google สำหรับใช้ Gemini
  google: {
    id: 'google',
    name: 'Google (Gemini)',
    clientId: 'project-a8112f3e-ee8c-45af-b73',
    authEndpoint: EXTERNAL_URLS.GOOGLE_AUTH,
    tokenEndpoint: EXTERNAL_URLS.GOOGLE_TOKEN,
    scopes: [
      'openid',
      'profile',
      'email',
      'https://www.googleapis.com/auth/generative-language',
    ],
    extraAuthParams: {
      access_type: 'offline',
      prompt: 'consent',
    },
    upstreamLLMProvider: 'google',
  },
  'chatgpt-pro': {
    id: 'chatgpt-pro',
    name: 'ChatGPT Plus/Pro',
    clientId: 'app_EMoamEEZ73f0CkXaXp7hrann',
    authEndpoint: EXTERNAL_URLS.OPENAI_AUTH,
    tokenEndpoint: EXTERNAL_URLS.OPENAI_TOKEN,
    scopes: ['openid', 'profile', 'email', 'offline_access'],
    extraAuthParams: {
      id_token_add_organizations: 'true',
      codex_cli_simplified_flow: 'true',
      originator: 'browseros',
    },
    upstreamLLMProvider: 'openai',
  },
  'github-copilot': {
    id: 'github-copilot',
    name: 'GitHub Copilot',
    clientId: 'Ov23li8tweQw6odWQebz',
    authEndpoint: EXTERNAL_URLS.GITHUB_DEVICE_CODE,
    tokenEndpoint: EXTERNAL_URLS.GITHUB_OAUTH_TOKEN,
    scopes: ['read:user'],
    upstreamLLMProvider: 'github-copilot',
    authFlow: 'device-code',
  },
  'qwen-code': {
    id: 'qwen-code',
    name: 'Qwen Code',
    clientId: 'f0304373b74a44d2b584a3fb70ca9e56',
    authEndpoint: EXTERNAL_URLS.QWEN_DEVICE_CODE,
    tokenEndpoint: EXTERNAL_URLS.QWEN_OAUTH_TOKEN,
    scopes: ['openid', 'profile', 'email', 'model.completion'],
    upstreamLLMProvider: 'qwen-code',
    authFlow: 'device-code',
    deviceCodeContentType: 'form',
    deviceCodeRequiresPKCE: true,
  },
}

export function getOAuthProvider(
  providerId: string,
): OAuthProviderConfig | undefined {
  return OAUTH_PROVIDERS[providerId]
}
