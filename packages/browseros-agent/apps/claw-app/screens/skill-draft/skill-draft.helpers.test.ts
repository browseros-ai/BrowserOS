/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { describe, expect, test } from 'bun:test'
import type { SessionSummary, SkillDraft } from '@browseros/claw-api'
import {
  canDraftSkills,
  canShowMakeSkill,
  draftProblem,
  editsFrom,
  isBusy,
  isDraftable,
  makeSkillState,
  newestDraftableId,
  resetDay,
  siteName,
  toSkillCreate,
} from './skill-draft.helpers'

const NOW = Date.parse('2026-11-18T12:00:00Z')

function task(overrides: Partial<SessionSummary> = {}): SessionSummary {
  return {
    sessionId: 's1',
    slug: 'claude-code',
    label: 'Claude Code',
    name: 'Browsed www.flipkart.com',
    startedAt: NOW,
    durationMs: 300_000,
    dispatchCount: 6,
    toolSequence: [],
    status: 'done',
    errorCount: 0,
    ...overrides,
  }
}

const draft: SkillDraft = {
  name: 'flipkart-phones-under-budget',
  title: 'Find phones under a budget on Flipkart',
  description: 'Lists the most popular phones under your budget.',
  site: 'flipkart.com',
  steps: [
    'Open flipkart.com and go to Mobiles.',
    'Set the price filter to a maximum of {budget}.',
    'If a brand is given, filter by {brand}.',
  ],
  inputs: [
    {
      name: 'budget',
      label: 'Highest price',
      example: '30000',
      kind: 'number',
    },
    { name: 'brand', label: 'Brand', example: 'Samsung', kind: 'text' },
  ],
  notes: ['The filter is in the left column.'],
  effects: 'read',
  confidence: 'high',
}

describe('who sees the button', () => {
  test('drafting comes with ultrafast', () => {
    expect(canDraftSkills(['claw.ultrafast'])).toBe(true)
    expect(canDraftSkills([])).toBe(false)
    expect(canDraftSkills(undefined)).toBe(false)
  })

  test('the button shows for a draftable task or one with a skill, on any plan', () => {
    expect(canShowMakeSkill(task())).toBe(true)
    expect(canShowMakeSkill(task({ skillName: 'neo-x' }))).toBe(true)
    expect(canShowMakeSkill(task({ status: 'failed' }))).toBe(false)
  })

  test('a click reopens a draft being written and waits while one is read or saved', () => {
    expect(isBusy('writing')).toBe(false)
    expect(isBusy('review')).toBe(true)
    expect(isBusy('saving')).toBe(true)
    expect(isBusy('idle')).toBe(false)
  })

  test('only finished or cleanly stopped tasks with steps and no skill', () => {
    expect(isDraftable(task())).toBe(true)
    expect(isDraftable(task({ status: 'cancelled', errorCount: 0 }))).toBe(true)
    expect(isDraftable(task({ status: 'cancelled', errorCount: 2 }))).toBe(
      false,
    )
    expect(isDraftable(task({ status: 'failed' }))).toBe(false)
    expect(isDraftable(task({ dispatchCount: 1 }))).toBe(false)
    expect(isDraftable(task({ skillName: 'neo-x' }))).toBe(false)
  })

  test('the newest draftable task is the one always shown', () => {
    const tasks = [
      task({ sessionId: 'live', status: 'live' }),
      task({ sessionId: 'saved', skillName: 'neo-x' }),
      task({ sessionId: 'first' }),
      task({ sessionId: 'second' }),
    ]
    expect(newestDraftableId(tasks)).toBe('first')
  })
})

describe('button state', () => {
  const base = { sheetOpen: false, drafting: false, saving: false, now: NOW }

  test('follows the shared draft status', () => {
    expect(makeSkillState({ ...base, status: undefined })).toBe('idle')
    expect(makeSkillState({ ...base, status: { status: 'none' } })).toBe('idle')
    expect(makeSkillState({ ...base, status: { status: 'writing' } })).toBe(
      'writing',
    )
    expect(makeSkillState({ ...base, status: { status: 'ready' } })).toBe(
      'draft',
    )
    expect(
      makeSkillState({ ...base, sheetOpen: true, status: { status: 'ready' } }),
    ).toBe('review')
    expect(
      makeSkillState({
        ...base,
        status: { status: 'saved', skillName: 'neo-x' },
      }),
    ).toBe('saved')
  })

  test('a pending request writes and a save wins over everything', () => {
    expect(makeSkillState({ ...base, drafting: true, status: undefined })).toBe(
      'writing',
    )
    expect(
      makeSkillState({ ...base, saving: true, status: { status: 'ready' } }),
    ).toBe('saving')
  })

  test('a limit holds until it resets, other failures can be retried', () => {
    const limited = {
      status: 'failed' as const,
      failure: {
        code: 'limit_reached' as const,
        scope: 'day' as const,
        resetsAt: '2026-11-19T00:00:00.000Z',
      },
    }
    expect(makeSkillState({ ...base, status: limited })).toBe('limited')
    expect(
      makeSkillState({
        ...base,
        now: Date.parse('2026-11-19T01:00:00Z'),
        status: limited,
      }),
    ).toBe('error')
    expect(
      makeSkillState({
        ...base,
        status: { status: 'failed', failure: { code: 'upstream_error' } },
      }),
    ).toBe('error')
  })
})

describe('what the sheet says', () => {
  test('limits point people to their agent and say when they reset', () => {
    const day = draftProblem({ code: 'limit_reached', scope: 'day' })
    expect(day.title).toBe('You’ve made 100 skills today.')
    expect(day.retry).toBe(false)
    const month = draftProblem({
      code: 'limit_reached',
      scope: 'month',
      resetsAt: '2026-12-01T00:00:00.000Z',
    })
    expect(month.detail).toBe('It resets on 1 December.')
    expect(month.agentLine).toContain('Your agent can still save one for you')
  })

  test('a failure offers a retry and the agent route', () => {
    const failed = draftProblem({ code: 'upstream_error' })
    expect(failed.title).toBe('We couldn’t write this skill right now.')
    expect(failed.retry).toBe(true)
    expect(failed.agentLine).toContain('If it keeps happening')
  })

  test('reset days are read in UTC', () => {
    expect(resetDay('2027-01-01T00:00:00.000Z')).toBe('1 January')
    expect(resetDay('not a date')).toBeUndefined()
  })

  test('site names read like a brand', () => {
    expect(siteName('www.flipkart.com')).toBe('Flipkart')
    expect(siteName(undefined)).toBeUndefined()
  })
})

describe('what a reviewed draft saves as', () => {
  test('carries the edits, the task and only the inputs a step still uses', () => {
    const edits = editsFrom(draft)
    edits.title = '  Cheap phones on Flipkart '
    edits.steps = [edits.steps[0] as string, edits.steps[1] as string, '  ']
    const body = toSkillCreate(draft, edits, 's1')
    expect(body).toEqual({
      name: 'neo-flipkart-phones-under-budget',
      description: 'Cheap phones on Flipkart',
      site: 'flipkart.com',
      steps: [
        'Open flipkart.com and go to Mobiles.',
        'Set the price filter to a maximum of {budget}.',
      ],
      learnedNotes: ['The filter is in the left column.'],
      sourceSessionId: 's1',
      summary: 'Lists the most popular phones under your budget.',
      inputs: [
        {
          name: 'budget',
          label: 'Highest price',
          example: '30000',
          kind: 'number',
        },
      ],
    })
  })
})
