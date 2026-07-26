/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { and, eq } from 'drizzle-orm'
import type { BrowserOsDatabase } from '../../db'
import { type OAuthTokenRow, oauthTokens } from '../../db/schema'
import type {
  OAuthStatus,
  OAuthTokenStore as OAuthTokenStoreContract,
  StoredOAuthTokens,
} from './token-manager'

/** Persists OAuth tokens in the BrowserOS Drizzle database for server-managed LLM providers. */
export class OAuthTokenStore implements OAuthTokenStoreContract {
  constructor(private readonly db: BrowserOsDatabase) {}

  upsertTokens(
    browserosId: string,
    provider: string,
    tokens: StoredOAuthTokens,
  ): void {
    const row: OAuthTokenRow = {
      browserosId,
      provider,
      accessToken: tokens.accessToken,
      refreshToken: tokens.refreshToken,
      expiresAt: tokens.expiresAt,
      email: tokens.email ?? null,
      accountId: tokens.accountId ?? null,
      updatedAt: Date.now(),
    }
    this.db
      .insert(oauthTokens)
      .values(row)
      .onConflictDoUpdate({
        target: [oauthTokens.browserosId, oauthTokens.provider],
        set: row,
      })
      .run()
  }

  getTokens(browserosId: string, provider: string): StoredOAuthTokens | null {
    const row = this.findRow(browserosId, provider)
    if (!row) return null
    return {
      accessToken: row.accessToken,
      refreshToken: row.refreshToken,
      expiresAt: row.expiresAt,
      email: row.email ?? undefined,
      accountId: row.accountId ?? undefined,
    }
  }

  deleteTokens(browserosId: string, provider: string): void {
    this.db.delete(oauthTokens).where(tokenKey(browserosId, provider)).run()
  }

  getStatus(browserosId: string, provider: string): OAuthStatus {
    const row = this.findRow(browserosId, provider)
    return {
      authenticated: row !== null,
      email: row?.email ?? undefined,
      provider,
    }
  }

  private findRow(browserosId: string, provider: string): OAuthTokenRow | null {
    return (
      this.db
        .select()
        .from(oauthTokens)
        .where(tokenKey(browserosId, provider))
        .get() ?? null
    )
  }
}

function tokenKey(browserosId: string, provider: string) {
  return and(
    eq(oauthTokens.browserosId, browserosId),
    eq(oauthTokens.provider, provider),
  )
}
