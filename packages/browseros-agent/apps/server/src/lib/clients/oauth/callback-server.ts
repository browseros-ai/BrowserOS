/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Temporary HTTP server on port 1455 for OAuth callbacks.
 * OpenAI's OAuth requires redirect_uri to use this specific port
 * (matching the Codex CLI client ID registration).
 */

import { OAUTH_CALLBACK_PORT } from '@browseros/shared/constants/ports'
import { logger } from '../../logger'
import type { OAuthTokenManager } from './token-manager'

export function startOAuthCallbackServer(tokenManager: OAuthTokenManager): {
  stop: () => void
} {
  const server = Bun.serve({
    port: OAUTH_CALLBACK_PORT,
    hostname: '127.0.0.1',
    fetch: async (req) => {
      const url = new URL(req.url)
      if (url.pathname !== '/auth/callback') {
        return new Response('Not found', { status: 404 })
      }

      const code = url.searchParams.get('code')
      const state = url.searchParams.get('state')
      const error = url.searchParams.get('error')

      if (error) {
        const description = url.searchParams.get('error_description') || error
        logger.warn('OAuth callback received error', { error, description })
        return htmlResponse(errorPage(description))
      }

      if (!code || !state) {
        return htmlResponse(errorPage('Missing authorization code or state'))
      }

      try {
        await tokenManager.handleCallback(code, state)

        // Always show success page — chrome-extension:// redirects are blocked by Chromium.
        // The extension polls /oauth/:provider/status and detects auth automatically.
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

  logger.info('OAuth callback server started', { port: OAUTH_CALLBACK_PORT })

  return {
    stop: () => {
      server.stop()
      logger.info('OAuth callback server stopped')
    },
  }
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
