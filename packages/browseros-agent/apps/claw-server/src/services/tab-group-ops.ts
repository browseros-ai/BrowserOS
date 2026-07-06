/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * v2 cockpit-owned tab-group orchestration. The dispatch path in
 * `mcp/register.ts` calls `ensureAgentTabGroup` after every
 * successful `tabs new` so the cockpit (not the agent) drives
 * `tab_groups create / update / close` on the agent's behalf.
 * The session-close path in `mcp/single-server.ts` calls
 * `closeAgentTabGroupForSession` before dropping the identity
 * record so the group container disappears in BrowserOS when the
 * agent disconnects.
 *
 * Both calls go through `executeTool` against the same
 * `BROWSER_TOOLS` catalogue the agent sees. The cockpit cannot just
 * call `tab_groups` over its MCP transport (that is the agent's
 * transport, not ours), so we dispatch the tool directly through
 * the framework, the same way the per-request handler does.
 *
 * Race handling: the FIRST `tabs new` per agent triggers a
 * `tab_groups create`. While that create is in flight, a SECOND
 * `tabs new` for the same agent would try to create a duplicate
 * group. We guard with an in-flight promise map keyed by agentId so
 * the second caller awaits the first call's result and then
 * dispatches an "add to existing group" call instead.
 */

import type { BrowserSession } from '@browseros/browser-core/core/session'
import { BROWSER_TOOLS } from '@browseros/browser-mcp/registry'
import type { ToolDefinition } from '@browseros/browser-mcp/tools/framework'
import { executeTool } from '@browseros/browser-mcp/tools/framework'
import { type TabGroupRecord, tabGroupTracker } from '../lib/agent-tab-groups'
import { logger } from '../lib/logger'

const TAB_GROUPS_TOOL: ToolDefinition = (() => {
  const t = BROWSER_TOOLS.find((tool) => tool.name === 'tab_groups')
  if (!t) {
    throw new Error('tab_groups tool not found in BROWSER_TOOLS')
  }
  return t
})()

interface EnsureInput {
  agentId: string
  slug: string
  pageId: number
  session: BrowserSession
  signal?: AbortSignal
}

interface CloseInput {
  agentId: string
  session: BrowserSession
}

interface ApplyTitleInput {
  agentId: string
  title: string
  session: BrowserSession | null
  signal?: AbortSignal
}

/** In-flight create promises keyed by agentId. See file comment. */
const inflight = new Map<string, Promise<void>>()

function extractFirstText(result: { content: unknown }): string {
  const arr = result.content as
    | Array<{ type: string; text?: string }>
    | undefined
  return arr?.[0]?.text ?? ''
}

function readGroupId(result: { structuredContent?: unknown }): string | null {
  const sc = result.structuredContent as
    | { group?: { groupId?: string } }
    | undefined
  return sc?.group?.groupId ?? null
}

function readWindowId(result: { structuredContent?: unknown }): number | null {
  const sc = result.structuredContent as
    | { group?: { windowId?: number } }
    | undefined
  return typeof sc?.group?.windowId === 'number' ? sc.group.windowId : null
}

async function dispatchCreate(input: EnsureInput, record: TabGroupRecord) {
  const result = await executeTool(
    TAB_GROUPS_TOOL,
    {
      action: 'create',
      pages: [input.pageId],
      title: record.title,
    },
    { session: input.session, signal: input.signal },
  )
  if (result.isError) {
    throw new Error(`tab_groups create failed: ${extractFirstText(result)}`)
  }
  const groupId = readGroupId(result)
  const windowId = readWindowId(result)
  if (!groupId) {
    throw new Error('tab_groups create returned no groupId')
  }
  tabGroupTracker.rememberGroup({
    agentId: input.agentId,
    groupId,
    windowId,
  })
  // Lock the colour separately. `tab_groups create` does not accept
  // a colour today; update does. A failure here does not invalidate
  // the group, just leaves the default colour.
  const colorResult = await executeTool(
    TAB_GROUPS_TOOL,
    { action: 'update', groupId, color: record.color },
    { session: input.session, signal: input.signal },
  )
  if (colorResult.isError) {
    logger.warn('agent tab group color lock failed', {
      agentId: input.agentId,
      groupId,
      error: extractFirstText(colorResult),
    })
  }
  logger.info('agent tab group created', {
    agentId: input.agentId,
    groupId,
    windowId,
    color: record.color,
  })
}

async function dispatchAddToGroup(input: EnsureInput, groupId: string) {
  const result = await executeTool(
    TAB_GROUPS_TOOL,
    {
      action: 'create',
      groupId,
      pages: [input.pageId],
    },
    { session: input.session, signal: input.signal },
  )
  if (!result.isError) return
  // Most likely cause: the user (or the agent) closed the group out
  // from under us. Blank the cockpit's record so the next `tabs new`
  // takes the create-from-scratch path.
  logger.warn('agent tab group add failed; resetting record', {
    agentId: input.agentId,
    groupId,
    error: extractFirstText(result),
  })
  tabGroupTracker.forgetGroup(input.agentId)
}

/**
 * Idempotent post-dispatch hook for `tabs new`. Creates the agent's
 * tab group on the first call, adds the new page to the existing
 * group on every subsequent call.
 */
export async function ensureAgentTabGroup(input: EnsureInput): Promise<void> {
  const record = tabGroupTracker.recordOpen({
    agentId: input.agentId,
    slug: input.slug,
    pageId: input.pageId,
  })

  // Serialise creates per agentId so two near-simultaneous `tabs new`
  // calls do not race into two `tab_groups create` calls.
  const existingInflight = inflight.get(input.agentId)
  if (existingInflight) {
    await existingInflight
  }

  const refreshed = tabGroupTracker.getByAgentId(input.agentId) ?? record
  if (refreshed.groupId === null) {
    const promise = dispatchCreate(input, refreshed).finally(() => {
      inflight.delete(input.agentId)
    })
    inflight.set(input.agentId, promise)
    try {
      await promise
    } catch (err) {
      logger.warn('agent tab group create failed', {
        agentId: input.agentId,
        error: err instanceof Error ? err.message : String(err),
      })
    }
    return
  }

  try {
    await dispatchAddToGroup(input, refreshed.groupId)
  } catch (err) {
    logger.warn('agent tab group add unexpected error', {
      agentId: input.agentId,
      error: err instanceof Error ? err.message : String(err),
    })
  }
}

/** Stores and, when possible, applies a display title to an agent tab group. */
export async function applyAgentTabGroupTitle(
  input: ApplyTitleInput,
): Promise<void> {
  tabGroupTracker.setTitle(input.agentId, input.title)
  try {
    await inflight.get(input.agentId)
  } catch (err) {
    logger.warn('agent tab group create finished before title apply', {
      agentId: input.agentId,
      error: err instanceof Error ? err.message : String(err),
    })
  }

  const record = tabGroupTracker.getByAgentId(input.agentId)
  if (!record?.groupId || !input.session) return

  try {
    const result = await executeTool(
      TAB_GROUPS_TOOL,
      { action: 'update', groupId: record.groupId, title: input.title },
      { session: input.session, signal: input.signal },
    )
    if (result.isError) {
      logger.warn('agent tab group title update failed', {
        agentId: input.agentId,
        groupId: record.groupId,
        error: extractFirstText(result),
      })
    }
  } catch (err) {
    logger.warn('agent tab group title update threw', {
      agentId: input.agentId,
      error: err instanceof Error ? err.message : String(err),
    })
  }
}

/**
 * Called from the transport's `onsessionclosed`. Decrements the ref
 * count; only the final session for a given agentId closes the group
 * in BrowserOS.
 */
export async function closeAgentTabGroupForAgent(
  input: CloseInput,
): Promise<void> {
  const record = tabGroupTracker.decrementSession(input.agentId)
  if (!record?.groupId) return
  try {
    const result = await executeTool(
      TAB_GROUPS_TOOL,
      { action: 'close', groupId: record.groupId },
      { session: input.session },
    )
    if (result.isError) {
      logger.warn('agent tab group close returned error', {
        agentId: input.agentId,
        groupId: record.groupId,
        error: extractFirstText(result),
      })
      return
    }
    logger.info('agent tab group closed', {
      agentId: input.agentId,
      groupId: record.groupId,
    })
  } catch (err) {
    logger.warn('agent tab group close threw', {
      agentId: input.agentId,
      error: err instanceof Error ? err.message : String(err),
    })
  }
}
