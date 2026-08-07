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

export type { AgentKey } from '../../domain/agent-key'
export { generateFunName } from './fun-names'
export type {
  ClientIdentity,
  IdentityService,
  RetainedIdentity,
} from './identity'
export {
  agentIdentityFromClient,
  agentKeyFromClient,
  createIdentityService,
  slugifyClientName,
} from './identity'
export {
  buildSessionGroupTitle,
  clientPrefixFromSlug,
  normalizeSmallName,
} from './naming'
