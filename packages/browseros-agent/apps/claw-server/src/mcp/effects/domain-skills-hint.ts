/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Surfaces domain-recipe files to the agent on navigate and on any
 * SPA URL transition. The effect delegates the actual discovery to
 * describeRecipesForHost so this file and the list_recipes tool
 * share one wire shape and one summary format.
 *
 * Recipes are context caching: they do not replace the browser tools,
 * they make each tool call cheaper to reason about. The synthetic text
 * block appended below carries a small nudge that reflects this.
 */

import {
  describeRecipesForHost,
  hostBucketFromUrl,
} from '../../services/recipes'
import type { ToolEffect } from '../dispatch'

/**
 * Per-session memory of the last host bucket surfaced to the caller.
 * Non-navigate tools only re-fire the annotation when the host bucket
 * changes since the last observation for that session, so a snapshot
 * loop on the same page does not restat the recipes directory on
 * every call.
 */
const lastHostBySession = new Map<string, string>()

/** Clears the per-session URL tracker. Test-only. */
export function clearDomainSkillsHintForTesting(): void {
  lastHostBySession.clear()
}

export const applyDomainSkillsHint: ToolEffect = ({ call, result }) => {
  if (result.isError) return
  if (!call.identity) return

  const url = extractUrl(result, call)
  if (!url) return
  const host = hostBucketFromUrl(url)
  if (!host) return

  const isNavigate = call.tool.name === 'navigate'
  const sessionKey = call.sessionId
  const lastHost = sessionKey ? lastHostBySession.get(sessionKey) : undefined
  if (!isNavigate && lastHost === host) return

  if (sessionKey) lastHostBySession.set(sessionKey, host)

  const discovery = describeRecipesForHost(call.identity.slug, host)
  const priorStructured =
    typeof result.structuredContent === 'object' &&
    result.structuredContent !== null
      ? (result.structuredContent as Record<string, unknown>)
      : {}
  const structuredContent = {
    ...priorStructured,
    domain_skills: {
      files: discovery.files,
      workspace_dir: discovery.workspace_dir,
      shared_dir: discovery.shared_dir,
      agent_dir: discovery.agent_dir,
    },
  }
  return {
    ...result,
    content: [...result.content, { type: 'text', text: discovery.summary }],
    structuredContent,
  }
}

function extractUrl(
  result: Parameters<ToolEffect>[0]['result'],
  call: Parameters<ToolEffect>[0]['call'],
): string | null {
  // Prefer the post-dispatch URL echoed by the tool (fresh after SPA
  // route changes and navigations); fall back to the destination the
  // caller asked for; finally the page snapshot captured at dispatch
  // time so non-URL-echoing tools (snapshot, act) still contribute.
  const fromResult = (result.structuredContent as { url?: unknown } | undefined)
    ?.url
  if (typeof fromResult === 'string' && fromResult.length > 0) return fromResult
  const fromArgs = (call.args as { url?: unknown } | null | undefined)?.url
  if (typeof fromArgs === 'string' && fromArgs.length > 0) return fromArgs
  const fromPage = call.pageSnapshot?.url
  if (typeof fromPage === 'string' && fromPage.length > 0) return fromPage
  return null
}
