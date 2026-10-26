/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Pure helpers for the task-detail screen. Isolated from the
 * screen component so they can be unit-tested against synthetic
 * dispatch rows without a React tree.
 */

import type { SessionScreenshot } from '@browseros/claw-api'
import type { DescribedStep } from '@/lib/step-sentences'
import type { ToolDispatchRow } from '@/modules/api/audit.hooks'
import type { ReplayAction } from '@/screens/replay/session-replay'

/** When each dispatch completed, by dispatch id. */
export function dispatchTimes(
  dispatches: readonly ToolDispatchRow[],
): Map<number, number> {
  return new Map(dispatches.map((row) => [row.dispatchId, row.createdAt]))
}

/** The step under the playhead: the last one that started at or before `at`. */
export function activeStepIndex(
  steps: readonly DescribedStep[],
  at: number | null,
): number {
  if (at === null) return -1
  let index = -1
  for (const [i, step] of steps.entries()) {
    if (step.firstAt > at) break
    index = i
  }
  return index
}

/** The replay action to seek to for a step: the first one recorded for it. */
export function actionForStep(
  step: DescribedStep,
  actions: readonly ReplayAction[],
): ReplayAction | undefined {
  const ids = new Set(step.dispatchIds)
  return actions.find(
    (action) =>
      action.frame.dispatchId != null && ids.has(action.frame.dispatchId),
  )
}

/** The picture for a step: one it took itself, else the latest taken before it. */
export function snapshotForStep(
  step: DescribedStep,
  dispatches: readonly ToolDispatchRow[],
  screenshots: readonly SessionScreenshot[],
): number | null {
  const ids = new Set(step.dispatchIds)
  const own = dispatches.filter(
    (row) => ids.has(row.dispatchId) && row.screenshotId != null,
  )
  const ownShot = own.at(-1)?.screenshotId
  if (ownShot != null) return ownShot
  let latest: SessionScreenshot | undefined
  for (const shot of screenshots) {
    if (
      shot.capturedAt <= step.firstAt &&
      (!latest || shot.capturedAt > latest.capturedAt)
    ) {
      latest = shot
    }
  }
  return latest?.screenshotId ?? null
}

export interface VisitedPage {
  url: string
  title: string
  host: string
}

/** Every web page the task touched, once each, in the order it first went there. */
/** A step's page as a web address without its fragment, or null for anything else. */
function webPage(raw: string | null | undefined): URL | null {
  if (!raw) return null
  let parsed: URL
  try {
    parsed = new URL(raw)
  } catch {
    return null
  }
  if (parsed.protocol !== 'http:' && parsed.protocol !== 'https:') return null
  parsed.hash = ''
  return parsed
}

/** The web page the task was on at its last step, even when it was visited before. */
export function lastPageUrl(
  dispatches: readonly ToolDispatchRow[],
): string | undefined {
  for (let index = dispatches.length - 1; index >= 0; index -= 1) {
    const page = webPage(dispatches[index]?.url)
    if (page) return page.toString()
  }
  return undefined
}

export function pagesVisited(
  dispatches: readonly ToolDispatchRow[],
): VisitedPage[] {
  const pages = new Map<string, VisitedPage>()
  for (const row of dispatches) {
    const parsed = webPage(row.url)
    if (!parsed) continue
    const url = parsed.toString()
    const host = parsed.hostname.replace(/^www\./, '')
    const existing = pages.get(url)
    if (existing) {
      if (row.title) existing.title = row.title
      continue
    }
    pages.set(url, { url, host, title: row.title || host })
  }
  return [...pages.values()]
}

/** "1:05" from the start of the task. */
export function offsetLabel(at: number, startedAt: number): string {
  const seconds = Math.max(0, Math.floor((at - startedAt) / 1000))
  return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, '0')}`
}
