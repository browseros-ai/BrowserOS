import { describe, expect, it } from 'bun:test'
import type { ToolDispatchRow } from '@/modules/api/audit.hooks'
import {
  describeLiveStep,
  describeStep,
  describeSteps,
  hostOf,
  isBehindTheScenes,
} from './step-sentences'

let nextId = 1

function call(
  toolName: string,
  options: {
    args?: Record<string, unknown>
    title?: string
    url?: string
    meta?: Record<string, unknown>
    at?: number
  } = {},
): ToolDispatchRow {
  const dispatchId = nextId++
  return {
    dispatchId,
    createdAt: options.at ?? 1_000 + dispatchId,
    slug: 'agent',
    label: 'Agent',
    sessionId: 'session-1',
    toolName,
    argsJson: options.args ? JSON.stringify(options.args) : undefined,
    resultMeta: options.meta ? JSON.stringify(options.meta) : undefined,
    title: options.title,
    url: options.url,
  }
}

const FLIGHTS = {
  title: 'San Francisco to New York | Google Flights',
  url: 'https://www.google.com/travel/flights/search',
}

describe('isBehindTheScenes', () => {
  it.each([
    'browser_profiles',
    'browser_target',
    'name_session',
    'wait',
    'screenshot',
    'helpers',
  ])('hides %s', (tool) => {
    expect(isBehindTheScenes(call(tool))).toBe(true)
  })

  it('hides listing calls on tabs, windows and tab groups', () => {
    expect(isBehindTheScenes(call('tabs', { args: { action: 'list' } }))).toBe(
      true,
    )
    expect(isBehindTheScenes(call('tabs'))).toBe(true)
    expect(
      isBehindTheScenes(call('windows', { args: { action: 'get' } })),
    ).toBe(true)
  })

  it('keeps calls that change what the person sees', () => {
    expect(isBehindTheScenes(call('tabs', { args: { action: 'new' } }))).toBe(
      false,
    )
    expect(isBehindTheScenes(call('act', { args: { kind: 'click' } }))).toBe(
      false,
    )
    expect(isBehindTheScenes(call('read'))).toBe(false)
  })
})

describe('describeStep', () => {
  it('names the page a new tab opened, preferring its title', () => {
    expect(
      describeStep(call('tabs', { args: { action: 'new' }, ...FLIGHTS })),
    ).toBe('Opened San Francisco to New York | Google Flights')
  })

  it('falls back to the host of the requested url', () => {
    expect(
      describeStep(
        call('tabs', {
          args: { action: 'new', url: 'https://www.google.com/travel/flights' },
        }),
      ),
    ).toBe('Opened google.com')
  })

  it.each([
    [{ kind: 'click' }, 'Clicked on Google Flights'],
    [{ kind: 'press', key: 'Enter' }, 'Pressed Enter on Google Flights'],
    [{ kind: 'scroll', direction: 'down' }, 'Scrolled down on Google Flights'],
    [{ kind: 'select' }, 'Picked an option on Google Flights'],
    [{ kind: 'dialog_dismiss' }, 'Closed a pop-up on Google Flights'],
  ])('describes act %j', (args, sentence) => {
    expect(describeStep(call('act', { args, title: 'Google Flights' }))).toBe(
      sentence,
    )
  })

  it('never repeats what was typed, because it can be a password', () => {
    const sentence = describeStep(
      call('act', {
        args: { kind: 'fill', text: 'hunter2', value: 'hunter2' },
        title: 'Sign in',
      }),
    )
    expect(sentence).toBe('Typed into Sign in')
    expect(sentence).not.toContain('hunter2')
  })

  it('reads plainly for reading tools', () => {
    expect(describeStep(call('snapshot', { title: 'Hacker News' }))).toBe(
      'Read Hacker News',
    )
    expect(describeStep(call('grep', { title: 'Hacker News' }))).toBe(
      'Searched Hacker News',
    )
  })

  it('reports errors and an operator stop in plain words', () => {
    expect(
      describeStep(
        call('act', {
          args: { kind: 'click' },
          url: 'https://billing.stripe.com/login',
          meta: { isError: true },
        }),
      ),
    ).toBe('Something went wrong on billing.stripe.com')
    expect(
      describeStep(
        call('read', {
          meta: { cancellationKind: 'cockpit.operator-cancelled' },
        }),
      ),
    ).toBe('You stopped the task')
  })

  it('describes the human-help and saved-task tools', () => {
    expect(describeStep(call('request_human_help'))).toBe('Asked you for help')
    expect(describeStep(call('save_skill'))).toBe('Saved this as a saved task')
  })

  it('shortens very long page titles', () => {
    const sentence = describeStep(call('read', { title: 'x'.repeat(200) }))
    expect(sentence.length).toBeLessThanOrEqual('Read '.length + 60)
    expect(sentence.endsWith('…')).toBe(true)
  })
})

describe('describeSteps', () => {
  it('drops behind-the-scenes calls and counts them', () => {
    const { steps, hiddenCount } = describeSteps([
      call('name_session'),
      call('browser_profiles', { args: { action: 'list' } }),
      call('tabs', { args: { action: 'new' }, ...FLIGHTS }),
      call('screenshot'),
      call('act', { args: { kind: 'click' }, ...FLIGHTS }),
    ])
    expect(hiddenCount).toBe(3)
    expect(steps.map((step) => step.sentence)).toEqual([
      'Opened San Francisco to New York | Google Flights',
      'Clicked on San Francisco to New York | Google Flights',
    ])
  })

  it('keeps behind-the-scenes calls, flagged, when asked to', () => {
    const { steps, hiddenCount } = describeSteps(
      [
        call('name_session'),
        call('act', { args: { kind: 'click' }, ...FLIGHTS }),
      ],
      { includeHidden: true },
    )
    expect(hiddenCount).toBe(1)
    expect(steps.map((step) => step.hidden)).toEqual([true, false])
  })

  it('folds repeated identical steps into one', () => {
    const first = call('read', { ...FLIGHTS, at: 5_000 })
    const second = call('read', FLIGHTS)
    const third = call('read', FLIGHTS)
    const { steps } = describeSteps([first, second, third])
    expect(steps).toHaveLength(1)
    expect(steps[0]?.dispatchIds).toEqual([
      first.dispatchId,
      second.dispatchId,
      third.dispatchId,
    ])
    expect(steps[0]?.firstAt).toBe(5_000)
  })

  it('flags error and cancelled steps', () => {
    const { steps } = describeSteps([
      call('act', { args: { kind: 'click' }, meta: { isError: true } }),
      call('read', {
        meta: { cancellationKind: 'cockpit.operator-cancelled' },
      }),
    ])
    expect(steps.map((step) => [step.isError, step.cancelled])).toEqual([
      [true, false],
      [false, true],
    ])
  })
})

describe('describeLiveStep', () => {
  it('says what the agent is doing on the page, in the present tense', () => {
    expect(
      describeLiveStep({
        title: 'Amazon.com : noise cancelling headphones',
        url: 'https://www.amazon.com/s',
        lastToolName: 'snapshot',
      }),
    ).toBe('Reading Amazon.com : noise cancelling headphones')
    expect(
      describeLiveStep({
        title: '',
        url: 'https://www.linkedin.com/login',
        lastToolName: 'request_human_help',
      }),
    ).toBe('Waiting for you on linkedin.com')
  })

  it('has a sensible line before any tab exists', () => {
    expect(describeLiveStep(undefined)).toBe('Getting started')
  })
})

describe('hostOf', () => {
  it('drops www and ignores invalid urls', () => {
    expect(hostOf('https://www.example.com/path')).toBe('example.com')
    expect(hostOf('not a url')).toBeUndefined()
  })
})
