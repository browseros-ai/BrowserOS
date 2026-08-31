/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { BrowserSession } from '@browseros/browser-core/core/session'
import {
  executeTool,
  type ToolResult,
} from '@browseros/browser-mcp/tools/framework'
import { tab_groups } from '@browseros/browser-mcp/tools/tab-groups'
import { logger } from '../../../lib/logger'
import type { ActiveConversationRun } from '../conversation-runs'

const GROUP_OPERATION_TIMEOUT_MS = 10_000
const TAB_GROUP_COLORS = [
  'grey',
  'blue',
  'red',
  'yellow',
  'green',
  'pink',
  'purple',
  'cyan',
  'orange',
] as const

interface GroupState {
  groupId: string
}

/**
 * Maintains one best-effort Chrome tab group per conversation and window.
 * Per-key queues serialize check/create/remember, preventing concurrent tab
 * opens from creating duplicate groups. State intentionally outlives runs.
 */
export class ConversationTabGroups {
  private readonly groups = new Map<string, GroupState>()
  private readonly queues = new Map<string, Promise<void>>()

  constructor(private readonly browserSession: BrowserSession) {}

  /** Starts detached work; the caller's browser-tool result is never delayed. */
  addCreatedPages(
    run: ActiveConversationRun,
    pageIds: readonly number[],
  ): void {
    void this.resolveWindowsAndQueue(run, pageIds).catch((error) => {
      logger.warn('Agent tab grouping failed', {
        conversationId: run.conversationId,
        error: errorText(error),
      })
    })
  }

  private async resolveWindowsAndQueue(
    run: ActiveConversationRun,
    pageIds: readonly number[],
  ): Promise<void> {
    await this.browserSession.pages.list()
    const byWindow = new Map<number, number[]>()
    for (const pageId of new Set(pageIds)) {
      const page = this.browserSession.pages.getInfo(pageId)
      if (!page || page.windowId === undefined) continue
      const pages = byWindow.get(page.windowId) ?? []
      pages.push(pageId)
      byWindow.set(page.windowId, pages)
    }

    for (const [windowId, pages] of byWindow) {
      this.enqueue(run, windowId, pages)
    }
  }

  private enqueue(
    run: ActiveConversationRun,
    windowId: number,
    pageIds: readonly number[],
  ): void {
    const key = groupKey(run.conversationId, windowId)
    const previous = this.queues.get(key) ?? Promise.resolve()
    const queued = previous
      .catch(() => undefined)
      .then(() => this.ensureGrouped(key, run, pageIds))
    this.queues.set(key, queued)
    void queued
      .catch((error) => {
        logger.warn('Agent tab group operation failed', {
          conversationId: run.conversationId,
          windowId,
          error: errorText(error),
        })
      })
      .finally(() => {
        if (this.queues.get(key) === queued) this.queues.delete(key)
      })
  }

  private async ensureGrouped(
    key: string,
    run: ActiveConversationRun,
    pageIds: readonly number[],
  ): Promise<void> {
    const existing = this.groups.get(key)
    if (existing) {
      const added = await this.dispatch({
        action: 'create',
        groupId: existing.groupId,
        pages: [...pageIds],
      })
      if (!added.isError) return

      // A user may delete the group between agent calls. Only clear a retained
      // reference after the authoritative list confirms it is gone.
      if ((await this.groupExists(existing.groupId)) !== false) {
        throw new Error(firstText(added))
      }
      this.groups.delete(key)
    }

    const presentation = run.tabGroup
    if (!presentation) return
    const created = await this.dispatch({
      action: 'create',
      pages: [...pageIds],
      title: presentation.title,
    })
    if (created.isError) throw new Error(firstText(created))
    const groupId = resultGroupId(created)
    if (!groupId) throw new Error('tab_groups create returned no groupId')
    this.groups.set(key, { groupId })

    // `create` cannot set a color, so lock the deterministic identity color in
    // a second best-effort operation after retaining the usable group id.
    const color = colorForKey(presentation.colorKey)
    const colored = await this.dispatch({
      action: 'update',
      groupId,
      color,
    })
    if (colored.isError) {
      logger.warn('Agent tab group color update failed', {
        conversationId: run.conversationId,
        groupId,
        color,
        error: firstText(colored),
      })
    }
  }

  private async groupExists(groupId: string): Promise<boolean | null> {
    try {
      const result = await this.dispatch({ action: 'list' })
      if (result.isError) return null
      const groups = asRecord(result.structuredContent)?.groups
      if (!Array.isArray(groups)) return null
      return groups.some((group) => asRecord(group)?.groupId === groupId)
    } catch {
      return null
    }
  }

  private async dispatch(args: Record<string, unknown>): Promise<ToolResult> {
    return await executeTool(tab_groups, args, {
      session: this.browserSession,
      signal: AbortSignal.timeout(GROUP_OPERATION_TIMEOUT_MS),
    })
  }
}

function resultGroupId(result: ToolResult): string | undefined {
  const group = asRecord(asRecord(result.structuredContent)?.group)
  return typeof group?.groupId === 'string' ? group.groupId : undefined
}

function colorForKey(value: string): (typeof TAB_GROUP_COLORS)[number] {
  let hash = 0x811c9dc5
  for (let index = 0; index < value.length; index += 1) {
    hash ^= value.charCodeAt(index)
    hash =
      (hash +
        ((hash << 1) +
          (hash << 4) +
          (hash << 7) +
          (hash << 8) +
          (hash << 24))) >>>
      0
  }
  return TAB_GROUP_COLORS[hash % TAB_GROUP_COLORS.length] ?? 'grey'
}

function groupKey(conversationId: string, windowId: number): string {
  return `${conversationId}:${windowId}`
}

function firstText(result: ToolResult): string {
  return result.content
    .filter(
      (item): item is { type: 'text'; text: string } =>
        item.type === 'text' && typeof item.text === 'string',
    )
    .map((item) => item.text)
    .join('\n')
}

function asRecord(value: unknown): Record<string, unknown> | undefined {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : undefined
}

function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}
