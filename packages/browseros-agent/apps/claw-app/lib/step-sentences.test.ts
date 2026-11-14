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
    args?: Record<string, unknown> | readonly unknown[]
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
  it('distinguishes a pending chooser from selecting files', () => {
    expect(
      describeStep(
        call('act', {
          args: { kind: 'click' },
          meta: { fileChooserOpen: true },
          ...FLIGHTS,
        }),
      ),
    ).toBe(
      'Opened a file chooser on San Francisco to New York | Google Flights',
    )
    expect(
      describeStep(
        call('upload', { args: { files: ['/vm/report.pdf'] }, ...FLIGHTS }),
      ),
    ).toBe('Selected files on San Francisco to New York | Google Flights')
    expect(
      describeStep(call('upload', { args: { files: [] }, ...FLIGHTS })),
    ).toBe(
      'Cleared file selection on San Francisco to New York | Google Flights',
    )
  })

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
    expect(
      describeStep(call('save_skill', { args: { name: 'x', flow: [] } })),
    ).toBe('Made this skill ultrafast')
    expect(
      describeStep(
        call('save_skill', { args: { name: 'x', patch: { step: 2 } } }),
      ),
    ).toBe('Repaired a step of this skill')
    expect(
      describeStep(call('run_skill', { args: { name: 'neo-shop-add' } })),
    ).toBe('Ran neo-shop-add the fast way')
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
      'Ran a browser script',
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

  it('picks the typing icon for typing and the click icon for other input', () => {
    expect(verbForCall(call('act', { args: { kind: 'fill' } }))).toBe('type')
    expect(verbForCall(call('input.click', { args: [3, 'e1'] }))).toBe('click')
    expect(verbForCall(call('nav.goto', { args: [3, 'https://x.y'] }))).toBe(
      'navigate',
    )
  })
})

describe('SDK run audit rows', () => {
  function sdkChild(
    toolName: string,
    identity: Pick<ToolDispatchRow, 'parentDispatchId' | 'tabId' | 'pageId'> = {
      parentDispatchId: 'run-1',
    },
  ): ToolDispatchRow {
    return {
      ...call(toolName, {
        args: ['[redacted]'],
        meta: { cancelled: false, isError: false },
      }),
      ...identity,
    }
  }

  it('keeps a failed run before its successful children in the normal rail', () => {
    const run = {
      ...call('run', {
        args: { code: '[redacted]' },
        meta: {
          cancelled: false,
          contentSummary: '1 block(s)',
          isError: true,
          sdkRunId: 'sdk-run-1',
        },
      }),
      dispatchKey: 'run-1',
    }
    const child = sdkChild('read', {
      parentDispatchId: run.dispatchKey,
      pageId: 2,
      tabId: 123,
    })
    const { steps, hiddenCount } = describeSteps([run, child])
    expect(hiddenCount).toBe(0)
    expect(steps.map((step) => step.sentence)).toEqual([
      'Something went wrong',
      'Read the page',
    ])
    expect(steps.map((step) => step.isError)).toEqual([true, false])
    expect(steps.map((step) => step.dispatchIds)).toEqual([
      [run.dispatchId],
      [child.dispatchId],
    ])
  })

  it('keeps two page openings on different tabs as separate steps', () => {
    const children = [123, 456].map((tabId) =>
      sdkChild('pages.newPage', { parentDispatchId: 'run-1', tabId }),
    )
    const { steps } = describeSteps(children)
    expect(steps.map((step) => step.sentence)).toEqual([
      'Opened a new page',
      'Opened a new page',
    ])
    expect(steps.map((step) => step.dispatchIds)).toEqual(
      children.map((child) => [child.dispatchId]),
    )
  })

  it('keeps redacted page openings separate when their tab is not recorded', () => {
    const children = [sdkChild('pages.newPage'), sdkChild('pages.newPage')]
    expect(
      describeSteps(children).steps.map((step) => step.dispatchIds),
    ).toEqual(children.map((child) => [child.dispatchId]))
  })

  it('still folds the same SDK step repeated on one tab in one run', () => {
    const children = [1, 2].map(() =>
      sdkChild('input.fill', {
        parentDispatchId: 'run-1',
        pageId: 2,
        tabId: 123,
      }),
    )
    const { steps } = describeSteps(children)
    expect(steps).toHaveLength(1)
    expect(steps[0]?.dispatchIds).toEqual(
      children.map((child) => child.dispatchId),
    )
  })

  it('does not fold the same tab step across hidden run parents', () => {
    const firstRun = {
      ...call('run', { args: { code: '[redacted]' } }),
      dispatchKey: 'run-1',
    }
    const secondRun = {
      ...call('run', { args: { code: '[redacted]' } }),
      dispatchKey: 'run-2',
    }
    const first = sdkChild('input.fill', {
      parentDispatchId: firstRun.dispatchKey,
      tabId: 123,
    })
    const second = sdkChild('input.fill', {
      parentDispatchId: secondRun.dispatchKey,
      tabId: 123,
    })
    const { steps, hiddenCount } = describeSteps([
      firstRun,
      first,
      secondRun,
      second,
    ])
    expect(hiddenCount).toBe(2)
    expect(steps.map((step) => step.dispatchIds)).toEqual([
      [first.dispatchId],
      [second.dispatchId],
    ])
  })

  it('keeps different SDK operations with the same generic sentence separate', () => {
    const children = ['sdk.operation', 'future.operation'].map((toolName) =>
      sdkChild(toolName, { parentDispatchId: 'run-1', tabId: 123 }),
    )
    expect(
      describeSteps(children).steps.map((step) => step.dispatchIds),
    ).toEqual(children.map((child) => [child.dispatchId]))
  })

  it.each([
    ['nav.goto', 'Opened a new page'],
    ['pages.newPage', 'Opened a new page'],
    ['nav.back', 'Went back to the page'],
    ['nav.forward', 'Went forward to the page'],
    ['nav.reload', 'Reloaded the page'],
    ['input.click', 'Clicked on the page'],
    ['input.fill', 'Typed into the page'],
    ['input.press', 'Pressed a key on the page'],
    ['input.hover', 'Moved to an item on the page'],
    ['input.selectOption', 'Picked an option on the page'],
    ['read', 'Read the page'],
    ['screenshot', 'Took a picture of the page'],
    ['pages.close', 'Closed a page it had opened'],
    ['sdk.operation', 'Worked in the browser'],
  ])('describes the redacted SDK child %s', (toolName, sentence) => {
    const child = {
      ...call(toolName, { args: ['[redacted]'] }),
      parentDispatchId: 'run-1',
    }
    expect(describeStep(child)).toBe(sentence)
    expect(
      describeSteps([child], { includeHidden: true }).steps[0]?.sentence,
    ).toBe(sentence)
  })

  it('tells a run through its redacted unknown child', () => {
    const run = {
      ...call('run', { args: { code: '[redacted]' } }),
      dispatchKey: 'run-1',
    }
    const child = {
      ...call('sdk.operation', { args: ['[redacted]'] }),
      parentDispatchId: run.dispatchKey,
    }
    const { steps, hiddenCount } = describeSteps([run, child])
    expect(hiddenCount).toBe(1)
    expect(steps.map((step) => step.sentence)).toEqual([
      'Worked in the browser',
    ])
    expect(steps[0]?.dispatchIds).toEqual([child.dispatchId])
  })

  it('describes a redacted run without child rows', () => {
    const run = call('run', { args: { code: '[redacted]' } })
    expect(describeStep(run)).toBe('Ran a browser script')
    const { steps, hiddenCount } = describeSteps([run])
    expect(hiddenCount).toBe(0)
    expect(steps.map((step) => step.sentence)).toEqual(['Ran a browser script'])
  })

  it('uses an optional task name without treating the page or agent label as one', () => {
    expect(
      describeStep(
        call('run', {
          args: { code: '[redacted]', taskName: '  Find flights  ' },
          ...FLIGHTS,
        }),
      ),
    ).toBe('Ran a browser script for Find flights')
    expect(describeStep(call('run', FLIGHTS))).toBe('Ran a browser script')
    expect(
      describeStep(call('run', { args: { taskName: 'x'.repeat(100) } })),
    ).toBe(`Ran a browser script for ${'x'.repeat(59)}…`)
  })

  it.each(['', '   ', null, 42, { name: 'Find flights' }])(
    'ignores a task name that is not a nonblank string: %j',
    (taskName) => {
      expect(describeStep(call('run', { args: { taskName } }))).toBe(
        'Ran a browser script',
      )
    },
  )

  it('still describes a run when its recorded arguments are malformed', () => {
    expect(describeStep({ ...call('run'), argsJson: '{' })).toBe(
      'Ran a browser script',
    )
  })

  it.each([
    { args: undefined },
    { args: { code: 'private script' } },
    { args: [3, { text: 'private text' }] },
  ])(
    'uses the same readable fallback for an unfamiliar operation with args %j',
    ({ args }) => {
      expect(describeStep(call('future.operation', { args }))).toBe(
        'Worked in the browser',
      )
      expect(
        describeStep(
          call('future.operation', { args, title: 'Google Flights' }),
        ),
      ).toBe('Worked on Google Flights')
    },
  )

  it.each(['constructor', 'toString', '__proto__'])(
    'treats the inherited property name %s as an unfamiliar operation',
    (toolName) => {
      for (const args of [undefined, [3, { text: '[redacted]' }]]) {
        const row = call(toolName, { args })
        expect(describeStep(row)).toBe('Worked in the browser')
        expect(verbForCall(row)).toBe('read')
        expect(isBehindTheScenes(row)).toBe(false)
      }
    },
  )

  it('keeps unfamiliar child dispatches in the displayed steps', () => {
    const run = {
      ...call('run', { args: { code: '[redacted]' } }),
      dispatchKey: 'run-1',
    }
    const children = ['future.operation', 'another.operation'].map(
      (toolName) => ({
        ...call(toolName, { args: { value: '[redacted]' }, ...FLIGHTS }),
        parentDispatchId: run.dispatchKey,
      }),
    )
    const { steps, hiddenCount } = describeSteps([run, ...children])
    expect(hiddenCount).toBe(0)
    expect(steps.map((step) => step.sentence)).toEqual([
      'Ran a browser script',
      'Worked on San Francisco to New York | Google Flights',
      'Worked on San Francisco to New York | Google Flights',
    ])
    expect(steps.flatMap((step) => step.dispatchIds)).toEqual([
      run.dispatchId,
      ...children.map((child) => child.dispatchId),
    ])
  })
})

describe('when neo takes a task over', () => {
  const STORE = 'https://shop.example.com/item/2-pack'
  const GOAL = 'Add the 2-pack to the cart'

  function pursue(outcome: string, at: number, durationMs: number) {
    return {
      ...call('pursue', {
        args: { page: 1, goal: GOAL },
        url: STORE,
        meta: { isError: false, outcome },
        at,
      }),
      durationMs,
    }
  }

  it('says neo took over the clicking, with the goal', () => {
    const step = pursue('completed', 1_000, 4_000)
    expect(describeStep(step)).toBe(
      'neo took over the clicking: “Add the 2-pack to the cart”',
    )
    expect(verbForCall(step)).toBe('browse')
  })

  it('marks where the agent carries on after a hand back', () => {
    const { steps } = describeSteps([
      pursue('handed_back', 1_000, 4_000),
      call('act', { args: { kind: 'click' }, url: STORE, at: 2_000 }),
      call('act', { args: { kind: 'fill' }, url: STORE, at: 6_000 }),
    ])

    expect(steps.map((step) => step.sentence)).toEqual([
      'neo took over the clicking: “Add the 2-pack to the cart”',
      'Clicked on shop.example.com',
      'Typed into shop.example.com',
    ])
    expect(steps.map((step) => step.afterHandBack ?? false)).toEqual([
      false,
      false,
      true,
    ])
    expect(steps[0]?.handedBack).toBe(true)
    expect(steps.some((step) => step.isError)).toBe(false)
  })

  it('marks nothing when neo finishes the task', () => {
    const { steps } = describeSteps([
      pursue('completed', 1_000, 4_000),
      call('read', { url: 'https://other.example.com', at: 6_000 }),
    ])

    expect(steps.some((step) => step.handedBack || step.afterHandBack)).toBe(
      false,
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
