/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * drizzle-kit config for the cockpit audit log.
 *
 * Runtime applies migrations programmatically on first DB
 * construction (see src/modules/db/migrator.ts). This config powers
 * `bunx drizzle-kit generate --name=<name>` for new migrations and
 * `drizzle-kit studio` for ad-hoc inspection.
 *
 * The `dbCredentials.url` here is a placeholder; the runtime resolves
 * the real path via `interfacePath('audit.sqlite')` so it always
 * tracks the operator's BROWSEROS_DIR override.
 */

import { defineConfig } from 'drizzle-kit'

export default defineConfig({
  dialect: 'sqlite',
  schema: './src/modules/db/schema',
  out: './drizzle',
  dbCredentials: {
    url: 'audit.sqlite',
  },
})
