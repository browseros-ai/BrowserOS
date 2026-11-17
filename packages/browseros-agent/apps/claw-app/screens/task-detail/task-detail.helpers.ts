/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Pure helpers for the task-detail screen. Isolated from the
 * screen component so they can be unit-tested against synthetic
 * dispatch rows without a React tree.
 */

import type {
  SessionPursueRun,
  SessionScreenshot,
  SessionUltrafastRun,
} from '@browseros/claw-api'
import type { DescribedStep } from '@/lib/step-sentences'
import type { ToolDispatchRow } from '@/modules/api/audit.hooks'
import {
  chipTokens,
  pagesLabel,
  ultrafastTime,
} from '@/screens/audit/audit.helpers'
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

/** The last replay action recorded for a step, where a folded step ends. */
export function lastActionForStep(
  step: DescribedStep,
  actions: readonly ReplayAction[],
): ReplayAction | undefined {
  const ids = new Set(step.dispatchIds)
  return actions.findLast(
    (action) =>
      action.frame.dispatchId != null && ids.has(action.frame.dispatchId),
  )
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

/** A row of "What happened": one step, or a whole neo run (the takeover and the steps neo took). */
export type RailItem =
  | { kind: 'step'; index: number }
  | { kind: 'run'; takeover: number; steps: number[] }

/**
 * The list's rows, with each neo run gathered behind the step where neo took over. A pursue call
 * that failed or was stopped from the cockpit stays an ordinary row, so its error still shows.
 */
export function railItems(steps: readonly DescribedStep[]): RailItem[] {
  const items: RailItem[] = []
  for (const [index, step] of steps.entries()) {
    const last = items.at(-1)
    if (step.neoOffsetMs !== undefined && last?.kind === 'run') {
      last.steps.push(index)
    } else if (step.neoRun && !step.isError && !step.cancelled) {
      items.push({ kind: 'run', takeover: index, steps: [] })
    } else {
      items.push({ kind: 'step', index })
    }
  }
  return items
}

/** The last step of a run, or the takeover when neo took none. */
export function runLastIndex(run: Extract<RailItem, { kind: 'run' }>): number {
  return run.steps.at(-1) ?? run.takeover
}

/** "+0.4s" under ten seconds, then "+12s": how long after the takeover a step came. */
export function offsetFromTakeover(offsetMs: number): string {
  const seconds = Math.max(0, offsetMs) / 1000
  return seconds < 10 ? `+${seconds.toFixed(1)}s` : `+${Math.round(seconds)}s`
}

/** Where each neo run sits on the scrubber, in activity seconds. */
export function neoRunMarks(
  steps: readonly DescribedStep[],
  actions: readonly ReplayAction[],
): { startAt: number; endAt: number }[] {
  const marks: { startAt: number; endAt: number }[] = []
  for (const item of railItems(steps)) {
    if (item.kind !== 'run') continue
    const takeover = steps[item.takeover]
    const last = steps[runLastIndex(item)]
    const start = takeover && actionForStep(takeover, actions)
    const end = last && lastActionForStep(last, actions)
    if (!start) continue
    marks.push({
      startAt: start.startAt,
      endAt: Math.max(
        start.startAt,
        end ? Math.max(end.startAt, end.completionAt) : start.startAt,
      ),
    })
  }
  return marks
}

/** A run of words where some are set as figures. */
export type Phrase = (string | { figure: string })[]

export interface UltrafastLineText {
  sentence: Phrase
  figures: { value: string; label: string }[]
}

/**
 * The task page's ultrafast line: one sentence of what neo did, then its figures.
 * Null when ultrafast did nothing in the task.
 */
export function ultrafastLineText(
  pursue: SessionPursueRun | undefined,
  ultrafast: SessionUltrafastRun | undefined,
  agent: string,
): UltrafastLineText | null {
  let sentence: Phrase
  if (pursue) {
    const time = { figure: ultrafastTime(pursue.durationMs) }
    const steps = pursue.steps.length
    if (pursue.outcome === 'completed') {
      sentence = ['neo finished the task on its own in ', time, '.']
    } else if (steps === 0) {
      sentence = [`neo handed the page back to ${agent} after `, time, '.']
    } else {
      sentence = [
        'neo did ',
        { figure: steps === 1 ? '1 step' : `${steps} steps` },
        ' in ',
        time,
        `, then handed the page back to ${agent}.`,
      ]
    }
  } else if (ultrafast) {
    const time = { figure: ultrafastTime(ultrafast.durationMs) }
    sentence =
      ultrafast.pagesRead > 0
        ? [
            'neo read the pages itself in ',
            time,
            `, so ${agent} didn’t have to.`,
          ]
        : ['neo ran it in ', time, '.']
  } else {
    return null
  }
  const figures: UltrafastLineText['figures'] = []
  if (ultrafast && ultrafast.tokensSaved > 0) {
    figures.push({
      value: chipTokens(ultrafast.tokensSaved),
      label: 'tokens saved',
    })
  }
  if (ultrafast && ultrafast.pagesRead > 0) {
    const [value, ...rest] = pagesLabel(ultrafast.pagesRead).split(' ')
    figures.push({ value: value ?? '', label: `${rest.join(' ')} read` })
  }
  if (pursue?.outcome === 'completed' && pursue.steps.length > 0) {
    const steps = pursue.steps.length
    figures.push({
      value: String(steps),
      label: steps === 1 ? 'step by neo' : 'steps by neo',
    })
  }
  return { sentence, figures }
}
