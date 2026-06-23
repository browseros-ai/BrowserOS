import { TIMEOUTS } from '@browseros/shared/constants/timeouts'
import type { BrowserSession } from '../browser/core/session'
import type { SnapshotDiff } from '../browser/core/snapshot/diff'
import { formatDiffResult } from './browser/diff-format'
import { formatSnapshotResult } from './browser/snapshot-format'

export type ContentItem =
  | { type: 'text'; text: string }
  | { type: 'image'; data: string; mimeType: string }

type PostAction =
  | SnapshotPostAction
  | { type: 'screenshot'; page: number }
  | DiffPostAction
  | { type: 'pages' }

type SnapshotPostAction = {
  type: 'snapshot'
  page: number
}

type DiffPostAction = {
  type: 'diff'
  page: number
  includeStructured?: boolean
}

export interface ToolResultMetadata {
  tabId?: number
}

export interface ToolResult {
  content: ContentItem[]
  isError?: boolean
  metadata?: ToolResultMetadata
  structuredContent?: unknown
}

export interface ToolResponseOptions {
  postActionTimeoutMs?: number
}

export class ToolResponse {
  private content: ContentItem[] = []
  private hasError = false
  private structured: unknown
  private postActions: PostAction[] = []
  private postActionTimeoutMs: number

  constructor(options: ToolResponseOptions = {}) {
    this.postActionTimeoutMs =
      options.postActionTimeoutMs ?? TIMEOUTS.TOOL_POST_ACTION
  }

  text(value: string): void {
    this.content.push({ type: 'text', text: value })
  }

  image(data: string, mimeType: string): void {
    this.content.push({ type: 'image', data, mimeType })
  }

  error(message: string): void {
    this.hasError = true
    this.content.push({ type: 'text', text: message })
  }

  data(key: string, value: unknown): void
  data(obj: Record<string, unknown>): void
  data(keyOrObj: string | Record<string, unknown>, value?: unknown): void {
    const current = isRecord(this.structured) ? this.structured : {}
    if (typeof keyOrObj === 'string') {
      current[keyOrObj] = value
      this.structured = current
      return
    }
    Object.assign(current, keyOrObj)
    this.structured = current
  }

  /** Merges a returned ToolResult into this response during incremental tool migration. */
  appendResult(result: ToolResult): void {
    this.content.push(...result.content)
    if (result.isError) this.hasError = true
    if ('structuredContent' in result) {
      if (isRecord(result.structuredContent)) {
        this.data(result.structuredContent)
      } else {
        this.structured = result.structuredContent
      }
    }
  }

  includeSnapshot(page: number): void {
    this.postActions.push({ type: 'snapshot', page })
  }

  includeScreenshot(page: number): void {
    this.postActions.push({ type: 'screenshot', page })
  }

  includeDiff(
    page: number,
    options: { includeStructured?: boolean } = {},
  ): void {
    this.postActions.push({
      type: 'diff',
      page,
      includeStructured: options.includeStructured,
    })
  }

  includePages(): void {
    this.postActions.push({ type: 'pages' })
  }

  private async runSessionPostAction(
    action: PostAction,
    session: BrowserSession,
  ): Promise<void> {
    switch (action.type) {
      case 'snapshot': {
        const { text } = await session.observe(action.page).snapshot()
        const origin = session.pages.getInfo(action.page)?.url ?? 'unknown'
        await this.appendSnapshotPostAction(action, text, origin)
        return
      }
      case 'screenshot': {
        const { session: pageSession } = await session.pages.getSession(
          action.page,
        )
        const result = await pageSession.Page.captureScreenshot({
          format: 'png',
          captureBeyondViewport: false,
        })
        this.text(`[Page ${action.page} screenshot]`)
        this.image(result.data, 'image/png')
        return
      }
      case 'diff': {
        const d = await session.observe(action.page).diff()
        const origin =
          d.afterUrl ?? session.pages.getInfo(action.page)?.url ?? 'unknown'
        await this.appendDiffPostAction(action, d, origin)
        return
      }
      case 'pages': {
        const pages = await session.pages.list()
        if (pages.length === 0) {
          this.text('[Open pages] None')
        } else {
          const lines = pages.map(
            (p) =>
              `  ${p.pageId}. ${p.title || '(untitled)'} — ${p.url}${p.isActive ? ' [ACTIVE]' : ''}`,
          )
          this.text(`[Open pages]\n${lines.join('\n')}`)
        }
        return
      }
    }
  }

  private async appendSnapshotPostAction(
    action: SnapshotPostAction,
    snapshot: string,
    origin: string,
  ): Promise<void> {
    const formatted = await formatSnapshotResult(snapshot, origin)
    this.text(`[Page ${action.page} snapshot]\n${formatted.text}`)
  }

  private async appendDiffPostAction(
    action: DiffPostAction,
    diff: SnapshotDiff,
    origin: string,
  ): Promise<void> {
    const formatted = await formatDiffResult(diff, origin)
    this.text(`[Page ${action.page} diff]\n${formatted.text}`)
    if (action.includeStructured) {
      this.data({
        changed: diff.changed,
        ...(diff.urlChanged && {
          urlChanged: true,
          beforeUrl: diff.beforeUrl,
          afterUrl: diff.afterUrl,
        }),
      })
    }
  }

  private async withTimeout<T>(task: Promise<T>): Promise<T> {
    let timeoutId: ReturnType<typeof setTimeout> | undefined
    try {
      return await Promise.race([
        task,
        new Promise<T>((_, reject) => {
          timeoutId = setTimeout(() => {
            reject(new Error('Post-action timed out'))
          }, this.postActionTimeoutMs)
        }),
      ])
    } finally {
      if (timeoutId !== undefined) clearTimeout(timeoutId)
    }
  }

  /** Builds a compact browser-tool result after running BrowserSession post-actions. */
  async buildForSession(session: BrowserSession): Promise<ToolResult> {
    if (this.postActions.length > 0) {
      this.text('\n--- Additional context (auto-included) ---')
    }

    for (const action of this.postActions) {
      try {
        await this.withTimeout(this.runSessionPostAction(action, session))
      } catch {
        // Post-action failure doesn't fail the tool
      }
    }
    return this.toResult()
  }

  toResult(): ToolResult {
    return {
      content: this.content,
      ...(this.hasError && { isError: true }),
      ...(this.structured !== undefined && {
        structuredContent: this.structured,
      }),
    }
  }
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}
