/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Process-wide identity service singleton. Construction is bound to
 * the package's clock (Date.now). Tests construct their own service
 * via `createIdentityService` with an injected clock; this singleton
 * is for the runtime route layer.
 */

import { createIdentityService } from './identity'

export const identityService = createIdentityService()

export type { ClientIdentity, IdentityService } from './identity'
export {
  agentIdentityFromClient,
  createIdentityService,
  fallbackSlugForSession,
  slugifyClientName,
} from './identity'
export {
  buildSessionGroupTitle,
  buildSessionNamePrompt,
  clientPrefixFromSlug,
  normalizeSmallName,
  sessionNameRequestedSchema,
} from './naming'
