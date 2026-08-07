/**
 * Fresh visual capture for a live MCP session. HTTP previews and post-dispatch
 * audit capture share this resolver so tab selection and ownership checks
 * cannot drift between the two consumers.
 */

import type { BrowserSession } from '@browseros/browser-core/core/session'
import { getBrowserSession } from '../lib/browser-session'
import { identityService } from '../lib/mcp-session'
import type { TabActivityRecord } from '../lib/tab-activity'
import { tabActivityRegistry } from '../lib/tab-activity'
import type { SessionTabRow } from '../modules/db/schema/session-tabs.sql'
import { getOpenSessionTab, listOpenSessionTabs } from './session-tabs'

const CAPTURE_TIMEOUT_MS = 2_000

export interface SessionVisualDependencies {
  isSessionLive(sessionId: string): boolean
  getBrowserSession(): BrowserSession | null
  listOpenSessionTabs(): SessionTabRow[]
  getOpenSessionTab(sessionId: string, tabId: number): SessionTabRow | null
  snapshotTabActivity(): TabActivityRecord[]
  captureTimeoutMs?: number
}

export interface SessionVisualService {
  capture(sessionId: string): Promise<Uint8Array | null>
}

interface CaptureCandidate {
  tabId: number
  pageId: number
  targetId: string
  lastActivityAt: number | null
}

export function createSessionVisualService(
  deps: SessionVisualDependencies,
): SessionVisualService {
  // Browser-core cannot cancel a screenshot already queued in CDP. Keep the
  // page guarded until that work actually settles so timed-out HTTP polling
  // cannot build an unbounded queue behind the per-page screenshot lock.
  const inFlightPageIds = new Set<number>()
  return {
    async capture(sessionId) {
      if (!deps.isSessionLive(sessionId)) return null
      const browser = deps.getBrowserSession()
      if (!browser) return null

      const pages = await browser.pages.list()
      const candidate = selectCaptureCandidate(
        sessionId,
        deps.listOpenSessionTabs(),
        pages,
        deps.snapshotTabActivity(),
      )
      if (!candidate) return null
      if (inFlightPageIds.has(candidate.pageId)) return null
      inFlightPageIds.add(candidate.pageId)

      const capturePromise = browser
        .screenshotForTarget(candidate.pageId, candidate.targetId, {
          format: 'jpeg',
          quality: 50,
          fullPage: false,
          annotate: false,
        })
        .finally(() => inFlightPageIds.delete(candidate.pageId))
      const capture = await withTimeout(
        capturePromise,
        deps.captureTimeoutMs ?? CAPTURE_TIMEOUT_MS,
      )
      if (!capture?.data) return null
      if (
        !deps.isSessionLive(sessionId) ||
        !deps.getOpenSessionTab(sessionId, candidate.tabId)
      ) {
        return null
      }

      const bytes = Buffer.from(capture.data, 'base64')
      return bytes.length > 0 ? bytes : null
    },
  }
}

export const sessionVisualService = createSessionVisualService({
  isSessionLive: (sessionId) => Boolean(identityService.getIdentity(sessionId)),
  getBrowserSession,
  listOpenSessionTabs,
  getOpenSessionTab,
  snapshotTabActivity: () => tabActivityRegistry.snapshot(),
})

function selectCaptureCandidate(
  sessionId: string,
  ownerships: readonly SessionTabRow[],
  pages: ReadonlyArray<{
    tabId: number
    pageId: number
    targetId: string
  }>,
  activities: readonly TabActivityRecord[],
): CaptureCandidate | null {
  const pagesByTabId = new Map(pages.map((page) => [page.tabId, page]))
  const activityByIncarnation = new Map(
    activities
      .filter((activity) => activity.sessionId === sessionId)
      .map((activity) => [
        activityKey(activity.tabId, activity.pageId, activity.targetId),
        activity.lastToolAt,
      ]),
  )
  const candidates = ownerships.flatMap((ownership): CaptureCandidate[] => {
    if (ownership.sessionId !== sessionId) return []
    const page = pagesByTabId.get(ownership.tabId)
    if (!page) return []
    return [
      {
        tabId: ownership.tabId,
        pageId: page.pageId,
        targetId: page.targetId,
        lastActivityAt:
          activityByIncarnation.get(
            activityKey(ownership.tabId, page.pageId, page.targetId),
          ) ?? null,
      },
    ]
  })
  candidates.sort(
    (left, right) =>
      (right.lastActivityAt ?? Number.NEGATIVE_INFINITY) -
        (left.lastActivityAt ?? Number.NEGATIVE_INFINITY) ||
      left.tabId - right.tabId ||
      left.pageId - right.pageId,
  )
  return candidates[0] ?? null
}

function activityKey(tabId: number, pageId: number, targetId: string): string {
  return `${tabId.toString()}\u0000${pageId.toString()}\u0000${targetId}`
}

async function withTimeout<T>(
  promise: Promise<T>,
  timeoutMs: number,
): Promise<T> {
  let timer: ReturnType<typeof setTimeout> | undefined
  const timeout = new Promise<never>((_, reject) => {
    timer = setTimeout(
      () => reject(new Error('session preview capture timed out')),
      timeoutMs,
    )
  })
  try {
    return await Promise.race([promise, timeout])
  } finally {
    if (timer !== undefined) clearTimeout(timer)
  }
}
