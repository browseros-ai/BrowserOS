/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import {
  type ClientCapabilities,
  type ElicitRequestFormParams,
  type ElicitResult,
  ErrorCode,
  McpError,
} from '@modelcontextprotocol/sdk/types.js'
import { getBrowserSession } from '../lib/browser-session'
import { logger } from '../lib/logger'
import {
  agentIdentityFromClient,
  buildSessionGroupTitle,
  buildSessionNamePrompt,
  type ClientIdentity,
  clientPrefixFromSlug,
  type IdentityService,
  identityService,
  normalizeSmallName,
  sessionNameRequestedSchema,
} from '../lib/mcp-session'
import { applyAgentTabGroupTitle } from '../services/tab-group-ops'

const ELICITATION_TIMEOUT_MS = 120_000
const ELICITATION_RETRY_DELAY_MS = 2_000

export interface SessionNamingServer {
  getClientCapabilities(): ClientCapabilities | undefined
  elicitInput(
    params: ElicitRequestFormParams,
    options: { timeout: number },
  ): Promise<ElicitResult>
}

export interface RequestSessionNamingDeps {
  identityService: Pick<IdentityService, 'getIdentity' | 'setSessionLabel'>
  getBrowserSession: typeof getBrowserSession
  applyTitle: typeof applyAgentTabGroupTitle
  delay: (ms: number) => Promise<void>
}

export interface RequestSessionNamingInput {
  server: SessionNamingServer
  sessionId: string
}

function defaultDelay(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms))
}

const defaultDeps: RequestSessionNamingDeps = {
  identityService,
  getBrowserSession,
  applyTitle: applyAgentTabGroupTitle,
  delay: defaultDelay,
}

async function elicitWithRetry(
  server: SessionNamingServer,
  prefix: string,
  delay: (ms: number) => Promise<void>,
): Promise<ElicitResult | null> {
  const params = {
    message: buildSessionNamePrompt(prefix),
    requestedSchema: sessionNameRequestedSchema,
  }
  try {
    return await server.elicitInput(params, { timeout: ELICITATION_TIMEOUT_MS })
  } catch (err) {
    if (err instanceof McpError && err.code === ErrorCode.RequestTimeout) {
      logger.info('mcp session naming elicitation unavailable', {
        error: err.message,
      })
      return null
    }
    await delay(ELICITATION_RETRY_DELAY_MS)
  }
  try {
    return await server.elicitInput(params, { timeout: ELICITATION_TIMEOUT_MS })
  } catch (err) {
    logger.info('mcp session naming elicitation unavailable', {
      error: err instanceof Error ? err.message : String(err),
    })
    return null
  }
}

function resolveLabel(result: ElicitResult): string | null {
  if (result.action !== 'accept') {
    logger.info('mcp session naming skipped', { action: result.action })
    return null
  }
  const rawName = result.content?.name
  if (typeof rawName !== 'string') return null
  const smallName = normalizeSmallName(rawName)
  return smallName.length > 0 ? smallName : null
}

function resolveIdentity(
  deps: RequestSessionNamingDeps,
  sessionId: string,
): ClientIdentity | null {
  return deps.identityService.getIdentity(sessionId)
}

/** Requests and applies a user-facing name for an initialized MCP session. */
export async function requestSessionNaming(
  input: RequestSessionNamingInput,
  deps: RequestSessionNamingDeps = defaultDeps,
): Promise<void> {
  if (!input.server.getClientCapabilities()?.elicitation) return

  const identity = resolveIdentity(deps, input.sessionId)
  if (!identity) return

  const { agentId, slug } = agentIdentityFromClient(identity)
  const prefix = clientPrefixFromSlug(slug)
  const result = await elicitWithRetry(input.server, prefix, deps.delay)
  if (!result) return

  const smallName = resolveLabel(result)
  if (!smallName) return

  deps.identityService.setSessionLabel(input.sessionId, smallName)
  await deps.applyTitle({
    agentId,
    title: buildSessionGroupTitle(prefix, smallName),
    session: deps.getBrowserSession(),
  })
}
