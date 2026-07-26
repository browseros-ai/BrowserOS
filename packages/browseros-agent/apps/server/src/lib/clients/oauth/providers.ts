/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { EXTERNAL_URLS } from '@browseros/shared/constants/urls'

const CHATGPT_PROVIDER_DISPLAY_NAME = 'ChatGPT'

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

const OAUTH_PROVIDERS: Record<string, OAuthProviderConfig> = {
  'chatgpt-pro': {
    id: 'chatgpt-pro',
    name: CHATGPT_PROVIDER_DISPLAY_NAME,
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
