/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Lazy OAuth callback server on port 1455.
 *
 * Port 1455 is required by OpenAI's Codex CLI OAuth client registration
 * (redirect_uri must be http://localhost:1455/auth/callback).
 *
 * Unlike the old implementation that bound the port at startup, this class:
 * - Only binds when the user initiates a PKCE login flow
 * - Sends GET /cancel to any existing server on the port first (Codex CLI pattern)
 * - Exposes /cancel so other instances can cancel us
 * - Releases the port after the callback arrives and no flows are pending
 */

import { OAUTH_CALLBACK_PORT } from '@browseros/shared/constants/ports'
import { logger } from '../../logger'
import type { OAuthTokenManager } from './token-manager'

const MAX_BIND_ATTEMPTS = 5
const RETRY_DELAY_MS = 300

export class OAuthCallbackServer {
  private server: ReturnType<typeof Bun.serve> | null = null
  private tokenManager: OAuthTokenManager | null = null

  setTokenManager(manager: OAuthTokenManager): void {
    this.tokenManager = manager
  }

  isRunning(): boolean {
    return this.server !== null
  }

  /**
   * Ensure the callback server is running on port 1455.
   * If the port is already held by another process, sends GET /cancel
   * to ask it to release, then retries.
   */
  async ensureRunning(): Promise<void> {
    if (this.server) return

    if (!this.tokenManager) {
      throw new Error('OAuth callback server not initialized')
    }

    let cancelSent = false

    for (let attempt = 1; attempt <= MAX_BIND_ATTEMPTS; attempt++) {
      try {
        this.bind()
        return
      } catch {
        if (!cancelSent) {
          cancelSent = true
          await this.sendCancel()
        }

        if (attempt < MAX_BIND_ATTEMPTS) {
          await sleep(RETRY_DELAY_MS)
        }
      }
    }

    throw new Error(
      `OAuth callback port ${OAUTH_CALLBACK_PORT} is in use by another process. ` +
        'Close other BrowserOS instances or CLI tools and try again.',
    )
  }

  /**
   * Stop the server and release port 1455.
   */
  stop(): void {
    if (this.server) {
      this.server.stop()
      this.server = null
      logger.info('OAuth callback server stopped', {
        port: OAUTH_CALLBACK_PORT,
      })
    }
  }

  private bind(): void {
    const tokenManager = this.tokenManager
    if (!tokenManager) {
      throw new Error('OAuth callback server not initialized')
    }

    this.server = Bun.serve({
      port: OAUTH_CALLBACK_PORT,
      hostname: '127.0.0.1',
      fetch: async (req) => {
        const url = new URL(req.url)

        // /cancel — let other instances ask us to release the port
        if (url.pathname === '/cancel') {
          logger.info('OAuth callback server received cancel request')
          // Schedule stop after responding
          queueMicrotask(() => this.stop())
          return new Response('Login cancelled', { status: 200 })
        }

        if (url.pathname !== '/auth/callback') {
          return new Response('Not found', { status: 404 })
        }

        const code = url.searchParams.get('code')
        const state = url.searchParams.get('state')
        const error = url.searchParams.get('error')

        if (error) {
          const description = url.searchParams.get('error_description') || error
          logger.warn('OAuth callback received error', {
            error,
            description,
          })
          return htmlResponse(errorPage(description))
        }

        if (!code || !state) {
          return htmlResponse(errorPage('Missing authorization code or state'))
        }

        try {
          await tokenManager.handleCallback(code, state)
          return htmlResponse(successPage())
        } catch (err) {
          logger.error('OAuth callback failed', {
            error: err instanceof Error ? err.message : String(err),
          })
          return htmlResponse(
            errorPage(
              err instanceof Error ? err.message : 'Authentication failed',
            ),
          )
        }
      },
    })

    logger.info('OAuth callback server started', {
      port: OAUTH_CALLBACK_PORT,
    })
  }

  /**
   * Send GET /cancel to any existing server on port 1455.
   * This politely asks the other process to release the port.
   * Follows the Codex CLI pattern (codex-rs/login/src/server.rs).
   */
  private async sendCancel(): Promise<void> {
    try {
      await fetch(`http://127.0.0.1:${OAUTH_CALLBACK_PORT}/cancel`, {
        signal: AbortSignal.timeout(2000),
      })
      logger.info('Sent cancel to existing OAuth callback server')
    } catch {
      // Server might not support /cancel or might not be running
    }
  }
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms))
}

function htmlResponse(html: string): Response {
  return new Response(html, {
    headers: { 'Content-Type': 'text/html; charset=utf-8' },
  })
}

function successPage(): string {
  return `<!DOCTYPE html>
<html><head><title>BrowserOS - Authentication Successful</title>
<style>body{font-family:system-ui;display:flex;align-items:center;justify-content:center;height:100vh;margin:0;background:#f8f9fa}
.card{text-align:center;padding:2rem;background:white;border-radius:12px;box-shadow:0 2px 8px rgba(0,0,0,0.1)}
h1{color:#22c55e;font-size:1.5rem}p{color:#6b7280}</style></head>
<body><div class="card"><h1>Authentication Successful</h1><p>You can close this tab and return to BrowserOS.</p></div></body></html>`
}

function errorPage(message: string): string {
  const escaped = message
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
  return `<!DOCTYPE html>
<html><head><title>BrowserOS - Authentication Failed</title>
<style>body{font-family:system-ui;display:flex;align-items:center;justify-content:center;height:100vh;margin:0;background:#f8f9fa}
.card{text-align:center;padding:2rem;background:white;border-radius:12px;box-shadow:0 2px 8px rgba(0,0,0,0.1)}
h1{color:#ef4444;font-size:1.5rem}p{color:#6b7280}</style></head>
<body><div class="card"><h1>Authentication Failed</h1><p>${escaped}</p><p>Please close this tab and try again.</p></div></body></html>`
}
