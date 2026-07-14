/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type {
  ClientCapabilities,
  ElicitRequestFormParams,
  ElicitResult,
} from '@modelcontextprotocol/sdk/types.js'
import type { AgentKey } from '../domain/agent-key'
import { getBrowserSession } from '../lib/browser-session'
import { logger } from '../lib/logger'
import {
  agentIdentityFromClient,
  agentKeyFromClient,
  buildSessionGroupTitle,
  buildSessionNamePrompt,
  type ClientIdentity,
  clientPrefixFromSlug,
  type IdentityService,
  identityService,
  normalizeSmallName,
  sessionNameRequestedSchema,
} from '../lib/mcp-session'
import { applyAgentTabGroupTitle } from './effects/tab-groups'

const ELICITATION_TIMEOUT_MS = 120_000

interface SessionNamingRequestOptions {
  timeout: number
  relatedRequestId?: string | number
  signal?: AbortSignal
}

export interface SessionNamingServer {
  getClientCapabilities(): ClientCapabilities | undefined
  elicitInput(
    params: ElicitRequestFormParams,
    options: SessionNamingRequestOptions,
  ): Promise<ElicitResult>
}

export interface RequestSessionNamingDeps {
  identityService: Pick<IdentityService, 'getIdentity' | 'setSessionLabel'>
  getBrowserSession: typeof getBrowserSession
  applyTitle: typeof applyAgentTabGroupTitle
}

export interface RequestSessionNamingInput {
  server: SessionNamingServer
  sessionId: string
}

export interface MaybeRequestSessionNamingInput
  extends RequestSessionNamingInput {
  requestId: string | number
}

interface ResolvedNamingIdentity {
  key: AgentKey
  prefix: string
}

const firedSessionIds = new Set<string>()
const pendingBySessionId = new Map<string, AbortController>()

const defaultDeps: RequestSessionNamingDeps = {
  identityService,
  getBrowserSession,
  applyTitle: applyAgentTabGroupTitle,
}

/** Issues the SDK call synchronously while its response stream is still open. */
function startElicitation(
  server: SessionNamingServer,
  prefix: string,
  options: SessionNamingRequestOptions,
): Promise<ElicitResult> {
  const params = {
    message: buildSessionNamePrompt(prefix),
    requestedSchema: sessionNameRequestedSchema,
  }
  try {
    return server.elicitInput(params, options)
  } catch (error) {
    return Promise.reject(error)
  }
}

function isAbort(error: unknown, signal?: AbortSignal): boolean {
  return (
    signal?.aborted === true ||
    (error instanceof Error && error.name === 'AbortError')
  )
}

async function awaitElicitation(
  firstAttempt: Promise<ElicitResult>,
  signal?: AbortSignal,
): Promise<ElicitResult | null> {
  try {
    return await firstAttempt
  } catch (error) {
    if (isAbort(error, signal)) return null
    logger.info('mcp session naming elicitation unavailable', {
      error: error instanceof Error ? error.message : String(error),
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
): ResolvedNamingIdentity | null {
  const identity: ClientIdentity | null =
    deps.identityService.getIdentity(sessionId)
  if (!identity) return null
  const { slug } = agentIdentityFromClient(identity)
  return {
    key: agentKeyFromClient(identity),
    prefix: clientPrefixFromSlug(slug),
  }
}

/** Normalizes and applies a completed session-name elicitation. */
async function finishSessionNaming(
  input: RequestSessionNamingInput,
  identity: ResolvedNamingIdentity,
  options: SessionNamingRequestOptions,
  deps: RequestSessionNamingDeps,
  firstAttempt: Promise<ElicitResult>,
): Promise<void> {
  const result = await awaitElicitation(firstAttempt, options.signal)
  if (!result || options.signal?.aborted) return

  const smallName = resolveLabel(result)
  if (!smallName || options.signal?.aborted) return

  deps.identityService.setSessionLabel(input.sessionId, smallName)
  await deps.applyTitle({
    key: identity.key,
    title: buildSessionGroupTitle(identity.prefix, smallName),
    session: deps.getBrowserSession(),
  })
}

/** Starts the first per-session naming request on the current request stream. */
export function maybeRequestSessionNaming(
  input: MaybeRequestSessionNamingInput,
  deps: RequestSessionNamingDeps = defaultDeps,
): Promise<void> {
  if (firedSessionIds.has(input.sessionId)) return Promise.resolve()

  if (!input.server.getClientCapabilities()?.elicitation) {
    firedSessionIds.add(input.sessionId)
    logger.info('mcp client lacks elicitation capability', {
      sessionId: input.sessionId,
    })
    return Promise.resolve()
  }

  const identity = resolveIdentity(deps, input.sessionId)
  if (!identity) return Promise.resolve()
  firedSessionIds.add(input.sessionId)

  const controller = new AbortController()
  pendingBySessionId.set(input.sessionId, controller)
  const options: SessionNamingRequestOptions = {
    timeout: ELICITATION_TIMEOUT_MS,
    relatedRequestId: input.requestId,
    signal: controller.signal,
  }
  const firstAttempt = startElicitation(input.server, identity.prefix, options)

  return finishSessionNaming(
    input,
    identity,
    options,
    deps,
    firstAttempt,
  ).finally(() => {
    if (pendingBySessionId.get(input.sessionId) === controller) {
      pendingBySessionId.delete(input.sessionId)
    }
  })
}

/** Aborts and resets session naming state during session teardown. */
export function cancelSessionNaming(sessionId: string): void {
  const controller = pendingBySessionId.get(sessionId)
  pendingBySessionId.delete(sessionId)
  firedSessionIds.delete(sessionId)
  controller?.abort()
}

/** Clears process-wide session naming state between tests. */
export function resetSessionNamingForTests(): void {
  for (const controller of pendingBySessionId.values()) controller.abort()
  pendingBySessionId.clear()
  firedSessionIds.clear()
}
