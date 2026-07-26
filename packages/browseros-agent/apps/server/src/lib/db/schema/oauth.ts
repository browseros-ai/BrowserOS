/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { InferInsertModel, InferSelectModel } from 'drizzle-orm'
import {
  index,
  integer,
  primaryKey,
  sqliteTable,
  text,
} from 'drizzle-orm/sqlite-core'

export const oauthTokens = sqliteTable(
  'oauth_tokens',
  {
    browserosId: text('browseros_id').notNull(),
    provider: text('provider').notNull(),
    accessToken: text('access_token').notNull(),
    refreshToken: text('refresh_token').notNull(),
    expiresAt: integer('expires_at').notNull(),
    email: text('email'),
    accountId: text('account_id'),
    updatedAt: integer('updated_at').notNull(),
  },
  (table) => [
    primaryKey({ columns: [table.browserosId, table.provider] }),
    index('oauth_tokens_browseros_id_idx').on(table.browserosId),
  ],
)

export type OAuthTokenRow = InferSelectModel<typeof oauthTokens>
export type NewOAuthTokenRow = InferInsertModel<typeof oauthTokens>
