/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { afterEach, beforeEach, describe, expect, it } from 'bun:test'
import { parseHTML } from 'linkedom'
import { act } from 'react'
import type { Root } from 'react-dom/client'
import { describeSteps } from '@/lib/step-sentences'
import type { ToolDispatchRow } from '@/modules/api/audit.hooks'
import { TaskSteps } from './TaskSteps'

const globalDescriptors = new Map(
  ['window', 'document', 'navigator', 'HTMLElement', 'Element', 'Node'].map(
    (name) => [name, Object.getOwnPropertyDescriptor(globalThis, name)],
  ),
)

let root: Root
let container: HTMLElement

beforeEach(async () => {
  const dom = parseHTML(
    '<!doctype html><html><body><div id="root"></div></body></html>',
  )
  const globals = {
    window: dom.window,
    document: dom.document,
    navigator: dom.window.navigator,
    HTMLElement: dom.window.HTMLElement,
    Element: dom.window.Element,
    Node: dom.window.Node,
  }
  for (const [name, value] of Object.entries(globals)) {
    Object.defineProperty(globalThis, name, {
      configurable: true,
      writable: true,
      value,
    })
  }
  Object.defineProperty(globalThis, 'IS_REACT_ACT_ENVIRONMENT', {
    configurable: true,
    writable: true,
    value: true,
  })
  // linkedom lays nothing out, so it has no scrolling; the list scrolls to the current step.
  Object.defineProperty(dom.window.HTMLElement.prototype, 'scrollTo', {
    configurable: true,
    value: () => undefined,
  })
  container = dom.document.getElementById('root') as unknown as HTMLElement
  const { createRoot } = await import('react-dom/client')
  root = createRoot(container)
})

afterEach(async () => {
  await act(async () => root.unmount())
  for (const [name, descriptor] of globalDescriptors) {
    if (descriptor) Object.defineProperty(globalThis, name, descriptor)
    else Reflect.deleteProperty(globalThis, name)
  }
  Reflect.deleteProperty(globalThis, 'IS_REACT_ACT_ENVIRONMENT')
})

const STORE = 'https://shop.example.com/item/2-pack'
let nextId = 1

function call(
  toolName: string,
  at: number,
  options: {
    args?: Record<string, unknown>
    meta?: Record<string, unknown>
    durationMs?: number
  } = {},
): ToolDispatchRow {
  return {
    dispatchId: nextId++,
    createdAt: at,
    slug: 'agent',
    label: 'Agent',
    sessionId: 'session-1',
    toolName,
    argsJson: options.args ? JSON.stringify(options.args) : undefined,
    resultMeta: JSON.stringify({ isError: false, ...options.meta }),
    durationMs: options.durationMs,
    url: STORE,
  }
}

function pursue(outcome: string, meta: Record<string, unknown> = {}) {
  return call('pursue', 1_000, {
    args: { page: 1, goal: 'Add the 2-pack to the cart' },
    meta: { outcome, ...meta },
    durationMs: 4_000,
  })
}

async function renderSteps(
  dispatches: ToolDispatchRow[],
  props: Partial<Parameters<typeof TaskSteps>[0]> = {},
) {
  const { steps, hiddenCount } = describeSteps(dispatches)
  await act(async () =>
    root.render(
      <TaskSteps
        steps={steps}
        activeIndex={-1}
        startedAt={0}
        hiddenCount={hiddenCount}
        showHidden={false}
        onShowHiddenChange={() => undefined}
        onSelect={() => undefined}
        agent="Claude"
        {...props}
      />,
    ),
  )
  return steps
}

/** The rows of the "What happened" list, as a person reads them. */
function rows(): string[] {
  return Array.from(container.querySelectorAll('ol > li')).map((li) =>
    (li.textContent ?? '').replace(/\s+/g, ' ').trim(),
  )
}

function button(name: string): HTMLElement | undefined {
  return Array.from(container.querySelectorAll('button')).find((element) =>
    (element.textContent ?? '').includes(name),
  ) as HTMLElement | undefined
}

async function click(element: HTMLElement | undefined) {
  await act(async () => {
    element?.dispatchEvent(
      new globalThis.window.Event('click', { bubbles: true }),
    )
  })
}

/** neo's run with `count` clicks, each 0.4s apart, then the agent typing. */
function longRun(count: number) {
  return [
    pursue('handed_back'),
    ...Array.from({ length: count }, (_, index) =>
      call(index % 2 ? 'act' : 'navigate', 1_100 + index * 400, {
        args: index % 2 ? { kind: 'click' } : { url: `${STORE}?p=${index}` },
      }),
    ),
    call('act', 6_000, { args: { kind: 'fill' } }),
  ]
}

describe('neo’s run in What happened', () => {
  it('opens with the takeover and goal, times neo’s steps, and closes with the hand back', async () => {
    await renderSteps([
      pursue('handed_back', { pagesRead: 4, pageTokens: 12_310 }),
      call('act', 2_000, { args: { kind: 'click' } }),
      call('act', 6_000, { args: { kind: 'fill' } }),
    ])

    expect(rows()).toEqual([
      'ultrafastneo took over the clicking“Add the 2-pack to the cart”0:01',
      'Clicked on shop.example.com+1.0s',
      'neo handed the page back to ClaudeSteps1Time4 secPages read4Tokens saved12kClaude carried on at 0:06',
      'Typed into shop.example.com0:06',
    ])
    expect(container.textContent).not.toContain('Back to your agent')
    expect(container.textContent).not.toContain('Something went wrong')
  })

  it('closes a run neo finished as finished', async () => {
    await renderSteps([
      pursue('completed'),
      call('act', 2_000, { args: { kind: 'click' } }),
    ])

    expect(container.textContent).toContain('neo finished it')
    expect(container.textContent).not.toContain('handed')
  })

  it('plays from a step neo took when it is chosen', async () => {
    const chosen: string[] = []
    await renderSteps(
      [pursue('handed_back'), call('act', 2_000, { args: { kind: 'click' } })],
      { onSelect: (step) => chosen.push(step.sentence) },
    )

    await click(button('Clicked on shop.example.com'))
    expect(chosen).toEqual(['Clicked on shop.example.com'])
  })

  it('chooses the run’s ending and marks it as the current row', async () => {
    const ends: number[] = []
    await renderSteps(
      [pursue('handed_back'), call('act', 2_000, { args: { kind: 'click' } })],
      { onSelectRunEnd: (takeover) => ends.push(takeover) },
    )
    await click(button('neo handed the page back'))
    expect(ends).toEqual([0])

    await renderSteps(
      [pursue('handed_back'), call('act', 2_000, { args: { kind: 'click' } })],
      { activeIndex: 1, chosenRunEnd: 0 },
    )
    const current = container.querySelector('[aria-current="step"]')
    expect(current?.textContent).toContain('neo handed the page back')
  })

  it('folds a long run to one row and opens it on click', async () => {
    await renderSteps(longRun(8))

    expect(container.textContent).toContain('8 steps by neo in 4 sec')
    expect(container.textContent).not.toContain('+0.1s')
    await click(button('Show them'))
    expect(container.textContent).toContain('+0.1s')
    expect(container.textContent).toContain('Fold neo’s 8 steps')
  })

  it('opens a folded run when the playing or chosen step is inside it', async () => {
    await renderSteps(longRun(8), { activeIndex: 3 })

    expect(container.textContent).toContain('+0.9s')
    expect(container.textContent).not.toContain('steps by neo')
  })

  it('keeps a short run open', async () => {
    await renderSteps(longRun(6))

    expect(container.textContent).not.toContain('Show them')
  })

  it('draws an ordinary task as before', async () => {
    await renderSteps([
      call('act', 2_000, { args: { kind: 'click' } }),
      call('act', 6_000, { args: { kind: 'fill' } }),
    ])

    expect(rows()).toEqual([
      'Clicked on shop.example.com0:02',
      'Typed into shop.example.com0:06',
    ])
    expect(container.textContent).not.toContain('ultrafast')
  })
})
