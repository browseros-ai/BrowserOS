/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { OAUTH_CALLBACK_PORT } from '@browseros/shared/constants/ports'
import { TIMEOUTS } from '@browseros/shared/constants/timeouts'
import { logger } from '../../logger'
import { getOAuthProvider } from './providers'
import type { OAuthTokenStore, StoredOAuthTokens } from './token-store'

interface PendingOAuthFlow {
  provider: string
  codeVerifier: string
  state: string
  redirectBackUrl?: string
  createdAt: number
}

interface OAuthTokenResponse {
  access_token: string
  refresh_token?: string
  expires_in: number
  id_token?: string
}

export class OAuthTokenManager {
  private readonly pendingFlows = new Map<string, PendingOAuthFlow>()
  private readonly refreshLocks = new Map<
    string,
    Promise<StoredOAuthTokens | null>
  >()

  constructor(
    private readonly store: OAuthTokenStore,
    private readonly browserosId: string,
  ) {}

  async generateAuthorizationUrl(
    providerId: string,
    redirectBackUrl?: string,
  ): Promise<string> {
    const provider = getOAuthProvider(providerId)
    if (!provider) throw new Error(`Unknown OAuth provider: ${providerId}`)

    const codeVerifier = generateCodeVerifier()
    const codeChallenge = await generateCodeChallenge(codeVerifier)
    const state = generateRandomState()

    this.pendingFlows.set(state, {
      provider: providerId,
      codeVerifier,
      state,
      redirectBackUrl,
      createdAt: Date.now(),
    })
    this.cleanExpiredFlows()

    const redirectUri = buildRedirectUri()
    const params = new URLSearchParams({
      response_type: 'code',
      client_id: provider.clientId,
      redirect_uri: redirectUri,
      code_challenge: codeChallenge,
      code_challenge_method: 'S256',
      scope: provider.scopes.join(' '),
      state,
      ...provider.extraAuthParams,
    })

    return `${provider.authEndpoint}?${params.toString()}`
  }

  async handleCallback(
    code: string,
    state: string,
  ): Promise<{ tokens: StoredOAuthTokens; redirectBackUrl?: string }> {
    const flow = this.pendingFlows.get(state)
    if (!flow) throw new Error('Invalid or expired OAuth state')
    if (Date.now() - flow.createdAt > TIMEOUTS.OAUTH_FLOW_TTL) {
      this.pendingFlows.delete(state)
      throw new Error('OAuth flow expired. Please try again.')
    }

    const provider = getOAuthProvider(flow.provider)
    if (!provider) throw new Error(`Unknown OAuth provider: ${flow.provider}`)

    const redirectUri = buildRedirectUri()
    const tokenResponse = await fetch(provider.tokenEndpoint, {
      method: 'POST',
      headers: { 'Content-Type': 'application/x-www-form-urlencoded' },
      body: new URLSearchParams({
        grant_type: 'authorization_code',
        client_id: provider.clientId,
        code,
        redirect_uri: redirectUri,
        code_verifier: flow.codeVerifier,
      }),
    })

    if (!tokenResponse.ok) {
      const error = await tokenResponse.text()
      logger.error('OAuth token exchange failed', {
        status: tokenResponse.status,
        error,
      })
      throw new Error(`Token exchange failed: ${tokenResponse.status}`)
    }

    const data = (await tokenResponse.json()) as OAuthTokenResponse
    if (!data.refresh_token) {
      logger.warn(
        'OAuth token response missing refresh_token — token refresh will not be available',
        {
          provider: flow.provider,
        },
      )
    }
    const { accountId, email } = parseAccessTokenClaims(data.access_token)

    const tokens: StoredOAuthTokens = {
      accessToken: data.access_token,
      refreshToken: data.refresh_token ?? '',
      expiresAt: Date.now() + data.expires_in * 1000,
      email,
      accountId,
    }

    this.store.upsertTokens(this.browserosId, flow.provider, tokens)
    this.pendingFlows.delete(state)

    logger.info('OAuth authentication successful', {
      provider: flow.provider,
      email,
    })

    return { tokens, redirectBackUrl: flow.redirectBackUrl }
  }

  // Mutex-protected refresh: concurrent callers share one in-flight refresh
  async refreshIfExpired(provider: string): Promise<StoredOAuthTokens | null> {
    const tokens = this.store.getTokens(this.browserosId, provider)
    if (!tokens) return null

    if (Date.now() < tokens.expiresAt - TIMEOUTS.OAUTH_TOKEN_EXPIRY_BUFFER) {
      return tokens
    }

    // If a refresh is already in progress, await it instead of starting another
    const existing = this.refreshLocks.get(provider)
    if (existing) return existing

    const refreshPromise = this.executeRefresh(provider, tokens)
    this.refreshLocks.set(provider, refreshPromise)

    try {
      return await refreshPromise
    } finally {
      this.refreshLocks.delete(provider)
    }
  }

  private async executeRefresh(
    provider: string,
    tokens: StoredOAuthTokens,
  ): Promise<StoredOAuthTokens> {
    if (!tokens.refreshToken) {
      this.store.deleteTokens(this.browserosId, provider)
      throw new Error(
        `${provider} session expired (no refresh token). Please re-login.`,
      )
    }

    const providerConfig = getOAuthProvider(provider)
    if (!providerConfig) {
      throw new Error(`Unknown OAuth provider: ${provider}`)
    }

    logger.debug('Refreshing OAuth token', { provider })

    const response = await fetch(providerConfig.tokenEndpoint, {
      method: 'POST',
      headers: { 'Content-Type': 'application/x-www-form-urlencoded' },
      body: new URLSearchParams({
        grant_type: 'refresh_token',
        client_id: providerConfig.clientId,
        refresh_token: tokens.refreshToken,
      }),
    })

    if (!response.ok) {
      logger.error('OAuth token refresh failed', {
        provider,
        status: response.status,
      })
      this.store.deleteTokens(this.browserosId, provider)
      const providerName = providerConfig.name
      throw new Error(`${providerName} session expired. Please re-login.`)
    }

    const data = (await response.json()) as OAuthTokenResponse
    const { accountId, email } = parseAccessTokenClaims(data.access_token)

    const refreshed: StoredOAuthTokens = {
      accessToken: data.access_token,
      refreshToken: data.refresh_token ?? tokens.refreshToken,
      expiresAt: Date.now() + data.expires_in * 1000,
      email: email ?? tokens.email,
      accountId: accountId ?? tokens.accountId,
    }

    this.store.upsertTokens(this.browserosId, provider, refreshed)
    return refreshed
  }

  getStatus(provider: string) {
    return this.store.getStatus(this.browserosId, provider)
  }

  deleteTokens(provider: string): void {
    this.store.deleteTokens(this.browserosId, provider)
  }

  private cleanExpiredFlows(): void {
    const now = Date.now()
    for (const [state, flow] of this.pendingFlows) {
      if (now - flow.createdAt > TIMEOUTS.OAUTH_FLOW_TTL) {
        this.pendingFlows.delete(state)
      }
    }
  }
}

function buildRedirectUri(): string {
  return `http://localhost:${OAUTH_CALLBACK_PORT}/auth/callback`
}

function generateCodeVerifier(): string {
  const bytes = crypto.getRandomValues(new Uint8Array(32))
  return base64UrlEncode(bytes)
}

async function generateCodeChallenge(verifier: string): Promise<string> {
  const encoder = new TextEncoder()
  const digest = await crypto.subtle.digest('SHA-256', encoder.encode(verifier))
  return base64UrlEncode(new Uint8Array(digest))
}

function generateRandomState(): string {
  const bytes = crypto.getRandomValues(new Uint8Array(16))
  return base64UrlEncode(bytes)
}

function base64UrlEncode(bytes: Uint8Array): string {
  const base64 = btoa(String.fromCharCode(...bytes))
  return base64.replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '')
}

// Extracts claims without signature verification — safe because the token
// comes directly from OpenAI's HTTPS token endpoint. Do not reuse for
// caller-supplied or externally-sourced tokens.
function parseAccessTokenClaims(accessToken: string): {
  accountId?: string
  email?: string
} {
  try {
    const parts = accessToken.split('.')
    if (parts.length !== 3) return {}
    const payload = JSON.parse(
      atob(parts[1].replace(/-/g, '+').replace(/_/g, '/')),
    )
    const authClaims = payload['https://api.openai.com/auth']
    const profileClaims = payload['https://api.openai.com/profile']
    return {
      accountId:
        authClaims?.chatgpt_account_id ??
        payload.chatgpt_account_id ??
        payload.account_id,
      email: profileClaims?.email ?? payload.email,
    }
  } catch {
    return {}
  }
}
