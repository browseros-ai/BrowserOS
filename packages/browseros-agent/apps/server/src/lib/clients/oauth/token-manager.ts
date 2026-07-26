/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { OAUTH_CALLBACK_PORT } from '@browseros/shared/constants/ports'
import { TIMEOUTS } from '@browseros/shared/constants/timeouts'
import { logger } from '../../logger'
import type { OAuthCallbackServer } from './callback-server'
import { getOAuthProvider, type OAuthProviderConfig } from './providers'

export interface StoredOAuthTokens {
  accessToken: string
  refreshToken: string
  expiresAt: number
  email?: string
  accountId?: string
}

export interface OAuthStatus {
  authenticated: boolean
  email?: string
  provider: string
}

export interface OAuthTokenStore {
  upsertTokens(
    browserosId: string,
    provider: string,
    tokens: StoredOAuthTokens,
  ): void
  getTokens(browserosId: string, provider: string): StoredOAuthTokens | null
  deleteTokens(browserosId: string, provider: string): void
  getStatus(browserosId: string, provider: string): OAuthStatus
}

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

export interface DeviceCodeResult {
  userCode: string
  verificationUri: string
  expiresIn: number
}

interface DeviceCodeResponse {
  device_code: string
  user_code: string
  verification_uri: string
  verification_uri_complete?: string
  expires_in: number
  interval: number
}

interface DeviceCodeTokenPollResponse {
  access_token?: string
  refresh_token?: string
  expires_in?: number
  error?: string
  interval?: number
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
    private readonly callbackServer: OAuthCallbackServer,
  ) {}

  // --- PKCE flow (ChatGPT) ---

  async generateAuthorizationUrl(
    providerId: string,
    redirectBackUrl?: string,
  ): Promise<string> {
    // Lazily start callback server — only needed for PKCE flow
    await this.callbackServer.ensureRunning()

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
    this.stopCallbackIfIdle()

    logger.info('OAuth authentication successful', {
      provider: flow.provider,
      email,
    })

    return { tokens, redirectBackUrl: flow.redirectBackUrl }
  }

  // --- Device Code flow (GitHub Copilot) ---

  private readonly activeDeviceFlows = new Set<string>()

  async startDeviceCodeFlow(providerId: string): Promise<DeviceCodeResult> {
    const provider = getOAuthProvider(providerId)
    if (!provider) throw new Error(`Unknown OAuth provider: ${providerId}`)

    // Cancel any existing flow — user may be retrying
    this.activeDeviceFlows.delete(providerId)

    // PKCE: generate verifier/challenge if provider requires it
    let codeVerifier: string | undefined
    const params: Record<string, string> = {
      client_id: provider.clientId,
      scope: provider.scopes.join(' '),
    }
    if (provider.deviceCodeRequiresPKCE) {
      codeVerifier = generateCodeVerifier()
      params.code_challenge = await generateCodeChallenge(codeVerifier)
      params.code_challenge_method = 'S256'
    }

    // Build request body (form-urlencoded or JSON based on provider)
    const useForm = provider.deviceCodeContentType === 'form'
    const response = await fetch(provider.authEndpoint, {
      method: 'POST',
      headers: {
        Accept: 'application/json',
        'Content-Type': useForm
          ? 'application/x-www-form-urlencoded'
          : 'application/json',
      },
      body: useForm
        ? new URLSearchParams(params).toString()
        : JSON.stringify(params),
    })

    // Detect WAF/captcha responses (HTML instead of JSON)
    const contentType = response.headers.get('content-type') ?? ''
    if (!contentType.includes('application/json')) {
      throw new Error(
        'Authentication service temporarily unavailable. Please try again in a few minutes.',
      )
    }

    if (!response.ok) {
      throw new Error(`Failed to request device code: ${response.status}`)
    }

    const data = (await response.json()) as DeviceCodeResponse

    // Some providers return 200 with an error payload
    const dataObj = data as unknown as Record<string, unknown>
    if ('error' in dataObj) {
      throw new Error(`Device code error: ${dataObj.error}`)
    }
    if (!data.device_code || !data.user_code) {
      throw new Error('Invalid device code response')
    }

    // Start background polling with error handling
    this.activeDeviceFlows.add(providerId)
    this.pollDeviceCode(
      providerId,
      provider,
      data.device_code,
      data.interval,
      data.expires_in,
      codeVerifier,
    ).finally(() => this.activeDeviceFlows.delete(providerId))

    return {
      userCode: data.user_code,
      verificationUri: data.verification_uri_complete ?? data.verification_uri,
      expiresIn: data.expires_in,
    }
  }

  private async pollDeviceCode(
    providerId: string,
    provider: OAuthProviderConfig,
    deviceCode: string,
    initialInterval: number,
    expiresIn: number,
    codeVerifier?: string,
  ): Promise<void> {
    let interval = initialInterval
    const deadline = Date.now() + expiresIn * 1000
    const useForm = provider.deviceCodeContentType === 'form'

    while (Date.now() < deadline) {
      await sleep(interval * 1000 + TIMEOUTS.DEVICE_CODE_POLL_SAFETY_MARGIN)

      try {
        const params: Record<string, string> = {
          client_id: provider.clientId,
          device_code: deviceCode,
          grant_type: 'urn:ietf:params:oauth:grant-type:device_code',
        }
        if (codeVerifier) params.code_verifier = codeVerifier

        const response = await fetch(provider.tokenEndpoint, {
          method: 'POST',
          headers: {
            Accept: 'application/json',
            'Content-Type': useForm
              ? 'application/x-www-form-urlencoded'
              : 'application/json',
          },
          body: useForm
            ? new URLSearchParams(params).toString()
            : JSON.stringify(params),
        })

        // WAF returned HTML instead of JSON — retry later
        const ct = response.headers.get('content-type') ?? ''
        if (!ct.includes('application/json')) {
          logger.warn('WAF blocked poll request, retrying', {
            provider: providerId,
          })
          continue
        }

        const data = (await response.json()) as DeviceCodeTokenPollResponse

        // Token received — store and return
        if (data.access_token) {
          const tokens: StoredOAuthTokens = {
            accessToken: data.access_token,
            refreshToken: data.refresh_token ?? '',
            expiresAt: data.expires_in
              ? Date.now() + data.expires_in * 1000
              : 0,
            email: undefined,
            accountId: undefined,
          }
          this.store.upsertTokens(this.browserosId, providerId, tokens)
          logger.info('Device code OAuth successful', { provider: providerId })
          return
        }

        // Handle polling errors per RFC 8628
        if (data.error === 'authorization_pending') continue
        if (data.error === 'slow_down') {
          interval = (data.interval ?? interval) + 5
          continue
        }
        if (data.error === 'expired_token' || data.error === 'access_denied') {
          logger.warn('Device code flow ended', {
            provider: providerId,
            error: data.error,
          })
          return
        }

        logger.warn('Unexpected device code poll response', {
          provider: providerId,
          error: data.error,
        })
        return
      } catch (err) {
        // Transient network error — loop continues to retry
        logger.warn('Device code poll request failed, retrying', {
          provider: providerId,
          error: err instanceof Error ? err.message : String(err),
        })
      }
    }

    logger.warn('Device code flow timed out', { provider: providerId })
  }

  // --- Token refresh ---

  async refreshIfExpired(provider: string): Promise<StoredOAuthTokens | null> {
    const tokens = this.store.getTokens(this.browserosId, provider)
    if (!tokens) return null

    // GitHub Copilot tokens never expire (expiresAt = 0)
    if (tokens.expiresAt === 0) return tokens

    if (Date.now() < tokens.expiresAt - TIMEOUTS.OAUTH_TOKEN_EXPIRY_BUFFER) {
      return tokens
    }

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

  // --- Shared ---

  // Store tokens provided by the extension (client-side auth flow)
  storeTokens(
    provider: string,
    params: { accessToken: string; refreshToken: string; expiresIn: number },
  ): void {
    const tokens: StoredOAuthTokens = {
      accessToken: params.accessToken,
      refreshToken: params.refreshToken,
      expiresAt: params.expiresIn ? Date.now() + params.expiresIn * 1000 : 0,
      email: undefined,
      accountId: undefined,
    }
    this.store.upsertTokens(this.browserosId, provider, tokens)
  }

  getTokens(provider: string): StoredOAuthTokens | null {
    return this.store.getTokens(this.browserosId, provider)
  }

  getStatus(provider: string) {
    return this.store.getStatus(this.browserosId, provider)
  }

  deleteTokens(provider: string): void {
    this.store.deleteTokens(this.browserosId, provider)
  }

  stopCallbackServer(): void {
    this.callbackServer.stop()
  }

  private stopCallbackIfIdle(): void {
    const hasPkceFlows = this.pendingFlows.size > 0
    if (!hasPkceFlows) {
      this.callbackServer.stop()
    }
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

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms))
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
