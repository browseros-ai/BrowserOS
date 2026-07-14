/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { BrowserSession } from '@browseros/browser-core/core/session'
import { BROWSER_TOOLS } from '@browseros/browser-mcp/registry'
import {
  executeTool,
  type ToolDefinition,
} from '@browseros/browser-mcp/tools/framework'
import { TIMEOUTS } from '@browseros/shared/constants/timeouts'
import type { AgentKey } from '../../domain/agent-key'
import { ownershipStore } from '../../domain/ownership'
import { colorForSlug } from '../../lib/agent-tab-groups'
import { logger } from '../../lib/logger'
import {
  buildSessionGroupTitle,
  type ClientIdentity,
  clientPrefixFromSlug,
} from '../../lib/mcp-session'
import type { ToolEffect } from '../dispatch'
import type { ToolResult } from '../register-fn'

const TAB_GROUPS_TOOL: ToolDefinition = (() => {
  const tool = BROWSER_TOOLS.find(
    (candidate) => candidate.name === 'tab_groups',
  )
  if (!tool) throw new Error('tab_groups tool not found in BROWSER_TOOLS')
  return tool
})()

interface EnsureAgentTabGroupInput {
  key: AgentKey
  identity: ClientIdentity
  pageId: number
  session: BrowserSession
  defaultTabGroupId: string | null
}

interface ApplyAgentTabGroupTitleInput {
  key: AgentKey
  title: string
  session: BrowserSession | null
}

const inflightCreates = new Map<AgentKey, Promise<void>>()

/** Creates or joins the durable group for a newly opened page. */
export async function ensureAgentTabGroup(
  input: EnsureAgentTabGroupInput,
): Promise<void> {
  if (input.defaultTabGroupId) return

  const existingCreate = inflightCreates.get(input.key)
  if (existingCreate) {
    await existingCreate
    const group = ownershipStore.groupOf(input.key)
    if (group) await addPageToGroup(input, group.id)
    return
  }

  const existingGroup = ownershipStore.groupOf(input.key)
  if (existingGroup) {
    await addPageToGroup(input, existingGroup.id)
    return
  }

  let create: Promise<void>
  create = createGroup(input)
    .catch((error) => {
      logger.warn('agent tab group create failed', {
        key: input.key,
        error: errorText(error),
      })
    })
    .finally(() => {
      if (inflightCreates.get(input.key) === create) {
        inflightCreates.delete(input.key)
      }
    })
  inflightCreates.set(input.key, create)
  await create
}

/** Stores a late title and applies it to a live durable group. */
export async function applyAgentTabGroupTitle(
  input: ApplyAgentTabGroupTitleInput,
): Promise<void> {
  await inflightCreates.get(input.key)
  const group = ownershipStore.groupOf(input.key)
  if (!group) return
  ownershipStore.updateGroup(input.key, {
    title: input.title,
    titleExplicit: true,
  })
  if (!input.session) return

  try {
    const result = await dispatchGroup(input.session, {
      action: 'update',
      groupId: group.id,
      title: input.title,
    })
    if (!result.isError) return
    // Update failures may be transient; the membership verify on the
    // next tabs new is the authoritative gone-group signal, so keep
    // the ref instead of risking a duplicate group.
    logger.warn('agent tab group title update failed', {
      key: input.key,
      groupId: group.id,
      error: firstText(result),
    })
  } catch (error) {
    logger.warn('agent tab group title update threw', {
      key: input.key,
      groupId: group.id,
      error: errorText(error),
    })
  }
}

/** Collapses the durable group after its final live session ends. */
export async function collapseAgentTabGroup(input: {
  key: AgentKey
  session: BrowserSession
}): Promise<void> {
  const group = ownershipStore.groupOf(input.key)
  if (!group || group.collapsed) return

  try {
    const result = await dispatchGroup(input.session, {
      action: 'update',
      groupId: group.id,
      collapsed: true,
    })
    if (result.isError) {
      logger.warn('agent tab group collapse failed', {
        key: input.key,
        groupId: group.id,
        error: firstText(result),
      })
      return
    }
    // The ref may have been replaced while the update was in flight;
    // only stamp the state if it still describes the group we updated.
    if (ownershipStore.groupOf(input.key)?.id === group.id) {
      ownershipStore.updateGroup(input.key, { collapsed: true })
    }
  } catch (error) {
    logger.warn('agent tab group collapse threw', {
      key: input.key,
      groupId: group.id,
      error: errorText(error),
    })
  }
}

/** Applies post-dispatch group creation and expansion without blocking the call. */
export const applyTabGroups: ToolEffect = ({ call, result }) => {
  if (result.isError || !call.key || !call.identity || !call.session) {
    return undefined
  }

  if (!call.flags.newPage) {
    void expandGroup(call.key, call.session)
    return undefined
  }
  const pageId = (result.structuredContent as { page?: number } | undefined)
    ?.page
  if (typeof pageId !== 'number') {
    void expandGroup(call.key, call.session)
    return undefined
  }
  let defaultTabGroupId = call.defaultTabGroupId
  if (
    defaultTabGroupId &&
    call.session.pages.getInfo(pageId)?.groupId !== defaultTabGroupId
  ) {
    ownershipStore.clearGroup(call.key)
    defaultTabGroupId = null
  }
  void expandGroup(call.key, call.session)
  void ensureAgentTabGroup({
    key: call.key,
    identity: call.identity,
    pageId,
    session: call.session,
    defaultTabGroupId,
  })
  return undefined
}

async function createGroup(input: EnsureAgentTabGroupInput): Promise<void> {
  const creationTitle = desiredTitle(input.identity, input.key)
  const color = colorForSlug(input.key)
  const result = await dispatchGroup(input.session, {
    action: 'create',
    pages: [input.pageId],
    title: creationTitle,
  })
  if (result.isError) throw new Error(firstText(result))

  const group = resultGroup(result)
  if (!group.id) throw new Error('tab_groups create returned no groupId')
  ownershipStore.setGroup(input.key, {
    id: group.id,
    windowId: group.windowId,
    color,
    title: creationTitle,
    titleExplicit: input.identity.sessionLabel !== null,
    collapsed: group.collapsed,
  })

  await lockGroupColor(input.key, group.id, color, input.session)
  const latestTitle = desiredTitle(input.identity, input.key)
  if (latestTitle !== creationTitle) {
    await applyCreationLateTitle(
      input.key,
      group.id,
      latestTitle,
      input.session,
    )
  }
  logger.info('agent tab group created', {
    key: input.key,
    groupId: group.id,
    windowId: group.windowId,
    color,
  })
}

async function addPageToGroup(
  input: EnsureAgentTabGroupInput,
  groupId: string,
): Promise<void> {
  try {
    const result = await dispatchGroup(input.session, {
      action: 'create',
      groupId,
      pages: [input.pageId],
    })
    if (!result.isError) return
    ownershipStore.clearGroup(input.key)
    logger.warn('agent tab group add failed', {
      key: input.key,
      groupId,
      error: firstText(result),
    })
  } catch (error) {
    logger.warn('agent tab group add threw', {
      key: input.key,
      groupId,
      error: errorText(error),
    })
  }
}

async function expandGroup(
  key: AgentKey,
  session: BrowserSession,
): Promise<void> {
  const group = ownershipStore.groupOf(key)
  if (!group?.collapsed) return
  try {
    const result = await dispatchGroup(session, {
      action: 'update',
      groupId: group.id,
      collapsed: false,
    })
    if (result.isError) {
      // Keep the ref: expand runs on every dispatch, so a transient
      // failure clearing it would spawn a duplicate group; the tabs-new
      // membership verify repairs a genuinely deleted group.
      logger.warn('agent tab group expand failed', {
        key,
        groupId: group.id,
        error: firstText(result),
      })
      return
    }
    if (ownershipStore.groupOf(key)?.id === group.id) {
      ownershipStore.updateGroup(key, { collapsed: false })
    }
  } catch (error) {
    logger.warn('agent tab group expand threw', {
      key,
      groupId: group.id,
      error: errorText(error),
    })
  }
}

async function lockGroupColor(
  key: AgentKey,
  groupId: string,
  color: ReturnType<typeof colorForSlug>,
  session: BrowserSession,
): Promise<void> {
  try {
    const result = await dispatchGroup(session, {
      action: 'update',
      groupId,
      color,
    })
    if (!result.isError) return
    logger.warn('agent tab group color lock failed', {
      key,
      groupId,
      error: firstText(result),
    })
  } catch (error) {
    logger.warn('agent tab group color lock threw', {
      key,
      groupId,
      error: errorText(error),
    })
  }
}

async function applyCreationLateTitle(
  key: AgentKey,
  groupId: string,
  title: string,
  session: BrowserSession,
): Promise<void> {
  try {
    const result = await dispatchGroup(session, {
      action: 'update',
      groupId,
      title,
    })
    if (result.isError) {
      logger.warn('agent tab group late title apply failed', {
        key,
        groupId,
        error: firstText(result),
      })
      return
    }
    ownershipStore.updateGroup(key, { title, titleExplicit: true })
  } catch (error) {
    logger.warn('agent tab group late title apply threw', {
      key,
      groupId,
      error: errorText(error),
    })
  }
}

function dispatchGroup(
  session: BrowserSession,
  args: Record<string, unknown>,
): Promise<ToolResult> {
  return executeTool(TAB_GROUPS_TOOL, args, {
    session,
    signal: AbortSignal.timeout(TIMEOUTS.TAB_GROUP_OP),
  })
}

function desiredTitle(identity: ClientIdentity, key: AgentKey): string {
  if (!identity.sessionLabel) return key
  return buildSessionGroupTitle(
    clientPrefixFromSlug(key),
    identity.sessionLabel,
  )
}

function resultGroup(result: ToolResult): {
  id: string | null
  windowId: number | null
  collapsed: boolean
} {
  const group = (
    result.structuredContent as
      | {
          group?: { groupId?: unknown; windowId?: unknown; collapsed?: unknown }
        }
      | undefined
  )?.group
  return {
    id: typeof group?.groupId === 'string' ? group.groupId : null,
    windowId: typeof group?.windowId === 'number' ? group.windowId : null,
    collapsed: group?.collapsed === true,
  }
}

function firstText(result: { content: unknown }): string {
  const content = result.content as
    | Array<{ type?: unknown; text?: unknown }>
    | undefined
  const first = content?.find(
    (item) => item.type === 'text' && typeof item.text === 'string',
  )
  return typeof first?.text === 'string' ? first.text : ''
}

function errorText(error: unknown): string {
  if (error instanceof Error) return error.message
  if (error !== null && typeof error === 'object' && 'content' in error) {
    return firstText(error as ToolResult)
  }
  return String(error)
}

/** Clears only in-flight orchestration state between tests. */
export function resetTabGroupEffectsForTesting(): void {
  inflightCreates.clear()
}
