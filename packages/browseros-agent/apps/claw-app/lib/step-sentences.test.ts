import { describe, expect, it } from 'bun:test'
import type { ToolDispatchRow } from '@/modules/api/audit.hooks'
import {
  describeIdleStep,
  describeLiveStep,
  describeStep,
  describeSteps,
  hostOf,
  isBehindTheScenes,
  verbForCall,
} from './step-sentences'

let nextId = 1

function call(
  toolName: string,
  options: {
    args?: Record<string, unknown> | unknown[]
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
    expect(describeStep(call('save_skill'))).toBe('Saved this as a skill')
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

  it('folds a read of the page the previous step worked on into that step', () => {
    const cart = {
      title: 'Amazon.in Shopping Cart',
      url: 'https://www.amazon.in/gp/cart/view.html',
    }
    const { steps } = describeSteps([
      call('act', { args: { kind: 'click' }, ...cart }),
      call('read', { ...cart, url: `${cart.url}?ref=nav` }),
      call('read', FLIGHTS),
    ])
    expect(steps.map((step) => step.sentence)).toEqual([
      'Clicked on Amazon.in Shopping Cart',
      'Read San Francisco to New York | Google Flights',
    ])
    expect(steps[0]?.dispatchIds).toHaveLength(2)
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

describe('calls made inside a run script', () => {
  const CART = {
    title: 'Amazon.in Shopping Cart',
    url: 'https://www.amazon.in/gp/cart/view.html',
  }

  it('reads them like the tools they mirror', () => {
    expect(
      describeStep(
        call('pages.newPage', { args: ['https://www.amazon.in/', null] }),
      ),
    ).toBe('Opened amazon.in')
    expect(
      describeStep(call('nav.goto', { args: [3, CART.url], ...CART })),
    ).toBe('Opened Amazon.in Shopping Cart')
    expect(
      describeStep(call('input.fill', { args: [3, 'e12', 'secret'] })),
    ).toBe('Typed into the page')
    expect(
      describeStep(call('input.press', { args: [3, 'Enter'], ...CART })),
    ).toBe('Pressed Enter on Amazon.in Shopping Cart')
    expect(describeStep(call('nav.reload', { args: [3], ...CART }))).toBe(
      'Reloaded Amazon.in Shopping Cart',
    )
  })

  it('reads page code as reading, unless it clicks', () => {
    const read = call('evaluate', {
      args: [3, { func: '() => document.title' }],
      ...CART,
    })
    const click = call('evaluate', {
      args: [3, { func: "() => document.querySelector('#buy').click()" }],
      ...CART,
    })
    expect(describeStep(read)).toBe('Read Amazon.in Shopping Cart')
    expect(verbForCall(read)).toBe('read')
    expect(describeStep(click)).toBe('Clicked on Amazon.in Shopping Cart')
    expect(verbForCall(click)).toBe('click')
  })

  it('hides script plumbing', () => {
    for (const tool of ['pages.close', 'pages.list', 'helpers.save', 'cdp']) {
      expect(isBehindTheScenes(call(tool, { args: [3] }))).toBe(true)
    }
    expect(
      isBehindTheScenes(call('wait', { args: [3, { for: 'text' }] })),
    ).toBe(true)
  })

  it('tells a script through its own calls, and keeps one that recorded none', () => {
    const { steps, hiddenCount } = describeSteps([
      call('run', { args: { code: 'await browser.open(url)' } }),
      call('pages.newPage', { args: ['https://www.amazon.in/', null] }),
      call('run', { args: { code: 'return 1 + 1' }, ...CART }),
    ])
    expect(hiddenCount).toBe(1)
    expect(steps.map((step) => step.sentence)).toEqual([
      'Opened amazon.in',
      'Worked on Amazon.in Shopping Cart',
    ])
  })

  it('gives behind-the-scenes calls plain words for when they are shown', () => {
    expect(describeStep(call('name_session'))).toBe('Named the task')
    expect(describeStep(call('tabs', { args: { action: 'list' } }))).toBe(
      'Checked the open tabs',
    )
    expect(describeStep(call('pages.close', { args: [3] }))).toBe(
      'Closed a page it had opened',
    )
  })

  it('names a decision-model goal in its own words', () => {
    expect(
      describeStep(
        call('pursue', {
          args: { goal: 'add a Sensodyne toothpaste to the cart.', page: 3 },
        }),
      ),
    ).toBe('Set out to add a Sensodyne toothpaste to the cart')
  })

  it('picks the typing icon for typing and the click icon for other input', () => {
    expect(verbForCall(call('act', { args: { kind: 'fill' } }))).toBe('type')
    expect(verbForCall(call('input.click', { args: [3, 'e1'] }))).toBe('click')
    expect(verbForCall(call('nav.goto', { args: [3, 'https://x.y'] }))).toBe(
      'navigate',
    )
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

describe('describeIdleStep', () => {
  it('names the page the agent last touched without saying it is busy', () => {
    expect(
      describeIdleStep({
        title: '',
        url: 'https://www.linkedin.com/login',
      }),
    ).toBe('Last active on linkedin.com')
  })

  it('has a sensible line when the agent never reached a page', () => {
    expect(describeIdleStep(undefined)).toBe('No recent activity')
  })
})

describe('hostOf', () => {
  it('drops www and ignores invalid urls', () => {
    expect(hostOf('https://www.example.com/path')).toBe('example.com')
    expect(hostOf('not a url')).toBeUndefined()
  })
})
