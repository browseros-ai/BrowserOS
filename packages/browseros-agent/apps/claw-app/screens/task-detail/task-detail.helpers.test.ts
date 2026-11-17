import { describe, expect, it } from 'bun:test'
import type { SessionScreenshot } from '@browseros/claw-api'
import type { DescribedStep } from '@/lib/step-sentences'
import type { ToolDispatchRow } from '@/modules/api/audit.hooks'
import type { ReplayAction } from '@/screens/replay/session-replay'
import {
  actionForStep,
  activeStepIndex,
  lastPageUrl,
  neoRunMarks,
  offsetFromTakeover,
  pagesVisited,
  railItems,
  snapshotForStep,
  ultrafastLineText,
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

describe('neo runs in the step list', () => {
  const takeover = {
    ...step(1_000, [1]),
    neoRun: {
      goal: 'Pick a show',
      outcome: 'handed_back' as const,
      stepCount: 2,
    },
  }
  const steps = [
    step(500, [0]),
    takeover,
    { ...step(1_400, [2]), neoOffsetMs: 400 },
    { ...step(3_100, [3]), neoOffsetMs: 2_100 },
    step(6_000, [4]),
  ]

  it('gathers each run behind its takeover', () => {
    expect(railItems(steps)).toEqual([
      { kind: 'step', index: 0 },
      { kind: 'run', takeover: 1, steps: [2, 3] },
      { kind: 'step', index: 4 },
    ])
  })

  it('times neo’s steps from the takeover', () => {
    expect(offsetFromTakeover(400)).toBe('+0.4s')
    expect(offsetFromTakeover(9_960)).toBe('+10.0s')
    expect(offsetFromTakeover(12_400)).toBe('+12s')
  })

  it('keeps a pursue call that failed as an ordinary row', () => {
    const failed = { ...takeover, isError: true }
    expect(
      railItems([step(500, [0]), failed]).map((item) => item.kind),
    ).toEqual(['step', 'step'])
  })

  it('marks the run on the scrubber from the takeover to neo’s last step', () => {
    const action = (dispatchId: number, startAt: number) =>
      ({
        frame: { dispatchId },
        startAt,
        completionAt: startAt,
      }) as ReplayAction
    expect(
      neoRunMarks(steps, [
        action(1, 40),
        action(2, 41),
        action(3, 43),
        action(4, 50),
      ]),
    ).toEqual([{ startAt: 40, endAt: 43 }])
  })

  it('ends the mark at the last call of a folded last step', () => {
    const action = (dispatchId: number, startAt: number) =>
      ({
        frame: { dispatchId },
        startAt,
        completionAt: startAt,
      }) as ReplayAction
    const folded = [
      ...steps.slice(0, 3),
      { ...steps[3], dispatchIds: [3, 5] },
      steps[4],
    ]
    expect(
      neoRunMarks(folded, [
        action(1, 40),
        action(2, 41),
        action(3, 43),
        action(5, 44),
        action(4, 50),
      ]),
    ).toEqual([{ startAt: 40, endAt: 44 }])
  })
})

describe('the ultrafast line on the task page', () => {
  const words = (text: ReturnType<typeof ultrafastLineText>) =>
    text?.sentence
      .map((part) => (typeof part === 'string' ? part : part.figure))
      .join('')

  it('says neo handed the page back, with the figures it saved', () => {
    const text = ultrafastLineText(
      {
        outcome: 'handed_back',
        durationMs: 5_662,
        steps: [{ action: 'click' }, { action: 'click' }, { action: 'scroll' }],
      },
      { durationMs: 5_662, pagesRead: 4, tokensSaved: 46_210 },
      'Claude',
    )
    expect(words(text)).toBe(
      'neo did 3 steps in 6 sec, then handed the page back to Claude.',
    )
    expect(text?.figures).toEqual([
      { value: '46k', label: 'tokens saved' },
      { value: '4', label: 'pages read' },
    ])
  })

  it('says neo finished on its own and adds its steps', () => {
    const text = ultrafastLineText(
      { outcome: 'completed', durationMs: 9_200, steps: [{ action: 'click' }] },
      { durationMs: 9_200, pagesRead: 1, tokensSaved: 900 },
      'Claude',
    )
    expect(words(text)).toBe('neo finished the task on its own in 9 sec.')
    expect(
      text?.figures.map((figure) => `${figure.value} ${figure.label}`),
    ).toEqual(['900 tokens saved', '1 page read', '1 step by neo'])
  })

  it('says neo read the pages itself on a skill run', () => {
    const text = ultrafastLineText(
      undefined,
      { durationMs: 15_354, pagesRead: 9, tokensSaved: 61_942 },
      'Claude',
    )
    expect(words(text)).toBe(
      'neo read the pages itself in 15 sec, so Claude didn’t have to.',
    )
  })

  it('says nothing on a task that ran the normal way', () => {
    expect(ultrafastLineText(undefined, undefined, 'Claude')).toBe(null)
  })
})
