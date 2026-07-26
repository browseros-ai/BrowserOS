/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { BrowserOsDatabase } from '../../db'
import { OAuthCallbackServer } from './callback-server'
import type { OAuthTokenManager } from './token-manager'
import { OAuthTokenManager as OAuthTokenManagerImpl } from './token-manager'
import { OAuthTokenStore } from './token-store'

let tokenManager: OAuthTokenManager | null = null

/** Initializes the process OAuth manager using the BrowserOS Drizzle database. */
export function initializeOAuth(
  db: BrowserOsDatabase,
  browserosId: string,
): OAuthTokenManager {
  shutdownOAuth()
  const store = new OAuthTokenStore(db)
  const callbackServer = new OAuthCallbackServer()
  tokenManager = new OAuthTokenManagerImpl(store, browserosId, callbackServer)
  callbackServer.setTokenManager(tokenManager)
  return tokenManager
}

export function getOAuthTokenManager(): OAuthTokenManager | null {
  return tokenManager
}

/** Stops the process OAuth manager and clears global access to provider tokens. */
export function shutdownOAuth(): void {
  tokenManager?.stopCallbackServer()
  tokenManager = null
}
