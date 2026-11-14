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

function pursue(outcome: string) {
  return call('pursue', 1_000, {
    args: { page: 1, goal: 'Add the 2-pack to the cart' },
    meta: { outcome },
    durationMs: 4_000,
  })
}

async function renderSteps(dispatches: ToolDispatchRow[]) {
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
      />,
    ),
  )
}

/** The rows of the "What happened" list, as a person reads them. */
function rows(): string[] {
  return Array.from(container.querySelectorAll('ol > li')).map((li) =>
    (li.textContent ?? '').replace(/\d+:\d+\s*$/, '').trim(),
  )
}

describe('TaskSteps with neo taking over', () => {
  it('shows the hand back as the agent carrying on, not as a failure', async () => {
    await renderSteps([
      pursue('handed_back'),
      call('act', 2_000, { args: { kind: 'click' } }),
      call('act', 6_000, { args: { kind: 'fill' } }),
    ])

    expect(rows()).toEqual([
      'neo took over the clicking: “Add the 2-pack to the cart”',
      'Clicked on shop.example.com',
      'Back to your agent',
      'Typed into shop.example.com',
    ])
    expect(container.textContent).not.toContain('Something went wrong')
  })

  it('still says who carries on when nothing was recorded after the hand back', async () => {
    await renderSteps([pursue('handed_back')])

    expect(rows()).toEqual([
      'neo took over the clicking: “Add the 2-pack to the cart”',
      'Back to your agent',
    ])
  })

  it('shows no hand back when neo finishes the task', async () => {
    await renderSteps([
      pursue('completed'),
      call('act', 2_000, { args: { kind: 'click' } }),
    ])

    expect(container.textContent).not.toContain('Back to your agent')
  })
})

describe('TaskSteps with SDK runs', () => {
  it('keeps a failed run before its successful children and marks it as failed', async () => {
    await renderSteps([
      {
        ...call('run', 1_000, { meta: { isError: true } }),
        dispatchKey: 'run-1',
        url: undefined,
        argsJson: '{"code":"[redacted]"}',
      },
      {
        ...call('observe.snapshot', 1_010),
        parentDispatchId: 'run-1',
        tabId: 42,
        url: undefined,
        argsJson: '["[redacted]"]',
      },
    ])

    expect(rows()).toEqual(['Something went wrong', 'Read the page'])
    const failed = container.querySelector('[data-step-index="0"]')
    expect(failed?.querySelector('.text-red-ink')).not.toBeNull()
    expect(failed?.querySelector('.lucide-triangle-alert')).not.toBeNull()
  })
})
