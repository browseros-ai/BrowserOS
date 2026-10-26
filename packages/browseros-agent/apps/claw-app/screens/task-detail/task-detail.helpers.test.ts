import { describe, expect, it } from 'bun:test'
import type { SessionScreenshot } from '@browseros/claw-api'
import type { DescribedStep } from '@/lib/step-sentences'
import type { ToolDispatchRow } from '@/modules/api/audit.hooks'
import type { ReplayAction } from '@/screens/replay/session-replay'
import {
  actionForStep,
  activeStepIndex,
  lastPageUrl,
  pagesVisited,
  snapshotForStep,
} from './task-detail.helpers'

function step(firstAt: number, dispatchIds: number[]): DescribedStep {
  return {
    sentence: `step at ${firstAt}`,
    verb: 'read',
    dispatchIds,
    firstAt,
    isError: false,
    cancelled: false,
    hidden: false,
  }
}

function row(
  dispatchId: number,
  createdAt: number,
  extra: Partial<ToolDispatchRow> = {},
): ToolDispatchRow {
  return {
    dispatchId,
    createdAt,
    slug: 'codex',
    label: 'Codex',
    sessionId: 's',
    toolName: 'read',
    ...extra,
  }
}

const steps = [step(1_000, [1]), step(5_000, [2, 3]), step(9_000, [4])]

describe('activeStepIndex', () => {
  it('follows the playhead, including the later calls folded into a step', () => {
    expect(activeStepIndex(steps, null)).toBe(-1)
    expect(activeStepIndex(steps, 500)).toBe(-1)
    expect(activeStepIndex(steps, 1_000)).toBe(0)
    expect(activeStepIndex(steps, 7_000)).toBe(1)
    expect(activeStepIndex(steps, 60_000)).toBe(2)
  })
})

describe('actionForStep', () => {
  it('seeks to the first recorded action of the step', () => {
    const actions = [3, 2, 4].map(
      (dispatchId, sourceIndex) =>
        ({ frame: { dispatchId }, startAt: sourceIndex }) as ReplayAction,
    )
    expect(actionForStep(steps[1] as DescribedStep, actions)?.startAt).toBe(0)
    expect(actionForStep(step(0, [99]), actions)).toBeUndefined()
  })
})

describe('snapshotForStep', () => {
  const shots: SessionScreenshot[] = [
    { screenshotId: 10, capturedAt: 900, toolName: 'navigate' },
    { screenshotId: 11, capturedAt: 4_000, toolName: 'act' },
  ]
  it('prefers the step’s own picture, else the latest one before it', () => {
    const dispatches = [row(2, 5_000), row(3, 5_500, { screenshotId: 12 })]
    expect(snapshotForStep(steps[1] as DescribedStep, dispatches, shots)).toBe(
      12,
    )
    expect(snapshotForStep(steps[2] as DescribedStep, dispatches, shots)).toBe(
      11,
    )
    expect(snapshotForStep(step(100, [7]), dispatches, shots)).toBeNull()
  })
})

describe('lastPageUrl', () => {
  it('is the page of the last step, even one visited earlier', () => {
    expect(
      lastPageUrl([
        row(1, 1, { url: 'https://a.example/' }),
        row(2, 2, { url: 'https://b.example/' }),
        row(3, 3, { url: 'https://a.example/#top' }),
        row(4, 4, { url: 'chrome://newtab/' }),
      ]),
    ).toBe('https://a.example/')
    expect(lastPageUrl([row(1, 1, { url: 'chrome://newtab/' })])).toBe(
      undefined,
    )
  })
})

describe('pagesVisited', () => {
  it('lists each web page once, keeps the latest title, skips non-web pages', () => {
    const pages = pagesVisited([
      row(1, 1, { url: 'https://www.google.com/travel/flights#x', title: '' }),
      row(2, 2, { url: 'chrome://newtab/' }),
      row(3, 3, {
        url: 'https://www.google.com/travel/flights',
        title: 'Flights',
      }),
      row(4, 4, { url: 'https://news.ycombinator.com/', title: 'HN' }),
    ])
    expect(pages).toEqual([
      {
        url: 'https://www.google.com/travel/flights',
        host: 'google.com',
        title: 'Flights',
      },
      {
        url: 'https://news.ycombinator.com/',
        host: 'news.ycombinator.com',
        title: 'HN',
      },
    ])
  })
})
