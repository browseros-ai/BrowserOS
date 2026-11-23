/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import {
  afterAll,
  afterEach,
  beforeEach,
  describe,
  expect,
  it,
  mock,
} from 'bun:test'
import { parseHTML } from 'linkedom'
import { act, type ReactNode } from 'react'
import type { Root } from 'react-dom/client'
import { MemoryRouter } from 'react-router'

// The beUI modal animates through motion and measures the DOM, which linkedom cannot do.
mock.module('@/components/motion/morphing-modal', () => ({
  MorphingModal: ({
    viewId,
    children,
  }: {
    viewId: string | null
    children: ReactNode
  }) =>
    viewId ? (
      <div role="dialog" data-view={viewId}>
        {children}
      </div>
    ) : null,
}))

const { SkillDraftSheet } = await import('./SkillDraftSheet')

afterAll(() => mock.restore())

const globalNames = [
  'window',
  'document',
  'navigator',
  'HTMLElement',
  'Node',
  'Event',
  'requestAnimationFrame',
  'cancelAnimationFrame',
  'getComputedStyle',
] as const
const globalDescriptors = new Map(
  globalNames.map((name) => [
    name,
    Object.getOwnPropertyDescriptor(globalThis, name),
  ]),
)

let root: Root
let container: HTMLElement

function define(name: string, value: unknown) {
  Object.defineProperty(globalThis, name, {
    configurable: true,
    writable: true,
    value,
  })
}

beforeEach(async () => {
  const dom = parseHTML(
    '<!doctype html><html><body><div id="root"></div></body></html>',
  )
  define('window', dom.window)
  define('document', dom.document)
  define('navigator', dom.window.navigator)
  define('HTMLElement', dom.window.HTMLElement)
  define('Node', dom.window.Node)
  define('Event', dom.window.Event)
  define('requestAnimationFrame', (run: FrameRequestCallback) =>
    setTimeout(() => run(0), 0),
  )
  define('cancelAnimationFrame', (id: number) => clearTimeout(id))
  define('getComputedStyle', () => ({ getPropertyValue: () => '' }))
  define('IS_REACT_ACT_ENVIRONMENT', true)
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

/** The sheet while a draft that started `ago` milliseconds back is being written. */
async function writingFor(ago: number) {
  await act(async () => {
    root.render(
      <MemoryRouter>
        <SkillDraftSheet
          open
          startedAt={Date.now() - ago}
          status={{ status: 'writing' }}
          source="From your Flipkart task, today at 18:39"
          saving={false}
          onClose={() => {}}
          onRetry={() => {}}
          onSave={() => {}}
        />
      </MemoryRouter>,
    )
  })
}

function text(): string {
  return (container.textContent ?? '').replace(/\s+/g, ' ')
}

function stepStates(): string[] {
  return [...container.querySelectorAll('ol li')].map((step) =>
    (step.textContent ?? '').trim(),
  )
}

describe('the sheet while a skill is written', () => {
  it('tests that it starts by reading the task', async () => {
    await writingFor(500)

    expect(stepStates()).toEqual([
      'In progress: Reading the task',
      'Still to come: Writing the steps',
      'Still to come: Checking it',
    ])
    expect(text()).toContain('0 s')
    expect(text()).not.toContain('Still writing, almost there.')
  })

  it('tests that it moves on to writing the steps and counts the seconds', async () => {
    await writingFor(5_200)

    expect(stepStates()).toEqual([
      'Done: Reading the task',
      'In progress: Writing the steps',
      'Still to come: Checking it',
    ])
    expect(text()).toContain('5 s')
    const bar = container.querySelector('[role="progressbar"]')
    expect(Number(bar?.getAttribute('aria-valuenow'))).toBeGreaterThan(30)
  })

  it('tests that a slow draft says it is almost there', async () => {
    await writingFor(11_000)

    expect(stepStates()[2]).toBe('In progress: Checking it')
    expect(text()).toContain('Still writing, almost there.')
    const bar = container.querySelector('[role="progressbar"]')
    expect(Number(bar?.getAttribute('aria-valuenow'))).toBeLessThan(100)
  })
})
