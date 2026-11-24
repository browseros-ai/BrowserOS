/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { afterEach, beforeEach, describe, expect, it, mock } from 'bun:test'
import type { PlanSavings, PlanSavingsPeriod } from '@browseros/claw-api'
import type { UseQueryResult } from '@tanstack/react-query'
import { parseHTML } from 'linkedom'
import { act } from 'react'
import type { Root } from 'react-dom/client'

const globalNames = [
  'window',
  'document',
  'navigator',
  'HTMLElement',
  'Node',
  'Event',
  'requestAnimationFrame',
  'cancelAnimationFrame',
  'ResizeObserver',
  'getComputedStyle',
  'localStorage',
] as const
const globalDescriptors = new Map(
  globalNames.map((name) => [
    name,
    Object.getOwnPropertyDescriptor(globalThis, name),
  ]),
)

const posthog = await import('@/modules/analytics/posthog')
const captured: string[] = []
mock.module('@/modules/analytics/posthog', () => ({
  ...posthog,
  capture: (event: string) => {
    captured.push(event)
  },
}))

const { UltrafastSummary } = await import('./UltrafastSummary')
const { SavingsHero } = await import('@/screens/pro/SavingsHero')
const { MemoryRouter } = await import('react-router')

function period(over: Partial<PlanSavingsPeriod> = {}): PlanSavingsPeriod {
  return {
    tokensSaved: 0,
    runs: 0,
    pagesRead: 0,
    durationMs: 0,
    siteTasks: 0,
    siteTasksFinished: 0,
    topSites: [],
    timeSavedMs: 0,
    tasksFinished: 0,
    runTimes: [],
    ...over,
  }
}

const PERIODS = {
  since: period({
    tokensSaved: 253_678,
    runs: 5,
    pagesRead: 38,
    timeSavedMs: 36 * 60_000,
  }),
  last30Days: period({ tokensSaved: 253_678, runs: 5, pagesRead: 38 }),
  last7Days: period({ tokensSaved: 68_000, runs: 4, pagesRead: 11 }),
}

const SINCE = new Date(2026, 10, 5, 9).getTime()

const SAVINGS: PlanSavings = {
  totals: { durationMs: 80_000, pagesRead: 38, tokensSaved: 253_678 },
  tasks: 5,
  latest: [],
  meter: {
    since: SINCE,
    tokensSaved: 253_678,
    runTimes: [],
    neoStepShare: 64,
    fastestRunMs: 3_095,
    periods: PERIODS,
  },
}

let root: Root
let body: HTMLElement

function define(name: string, value: unknown) {
  Object.defineProperty(globalThis, name, {
    configurable: true,
    writable: true,
    value,
  })
}

function query(
  over: Partial<UseQueryResult<PlanSavings>>,
): UseQueryResult<PlanSavings> {
  return {
    data: SAVINGS,
    isPending: false,
    isError: false,
    refetch: async () => ({}),
    ...over,
  } as UseQueryResult<PlanSavings>
}

async function render(savings: UseQueryResult<PlanSavings>) {
  await act(async () =>
    root.render(
      <MemoryRouter>
        <UltrafastSummary savings={savings} />
      </MemoryRouter>,
    ),
  )
}

function text(): string {
  return (body.textContent ?? '').replace(/\s+/g, ' ')
}

beforeEach(async () => {
  const dom = parseHTML(
    '<!doctype html><html><body><div id="root"></div></body></html>',
  )
  Object.defineProperty(dom.window, 'matchMedia', {
    configurable: true,
    value: () => ({ matches: true }),
  })
  define('window', dom.window)
  define('document', dom.document)
  define('navigator', dom.window.navigator)
  define('HTMLElement', dom.window.HTMLElement)
  define('Node', dom.window.Node)
  define('Event', dom.window.Event)
  define('requestAnimationFrame', (cb: FrameRequestCallback) =>
    setTimeout(() => cb(performance.now()), 16),
  )
  define('cancelAnimationFrame', (id: number) => clearTimeout(id))
  define(
    'ResizeObserver',
    class {
      observe() {}
      disconnect() {}
    },
  )
  define('getComputedStyle', () => ({ direction: 'ltr' }))
  define('IS_REACT_ACT_ENVIRONMENT', true)
  body = dom.document.body as unknown as HTMLElement
  const container = dom.document.getElementById(
    'root',
  ) as unknown as HTMLElement
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

function at(tokens: number): PlanSavings {
  return {
    ...SAVINGS,
    meter: {
      ...SAVINGS.meter,
      since: SINCE,
      runTimes: [],
      tokensSaved: tokens,
      periods: PERIODS,
    },
  }
}

function details(): Element | undefined {
  return [...body.querySelectorAll('button')].find((button) =>
    button.textContent?.includes('Details'),
  )
}

function facts(): string[] {
  return [...body.querySelectorAll('dl > div')].map((fact) =>
    (fact.textContent ?? '').replace(/\s+/g, ' '),
  )
}

describe('the ultrafast card on Home', () => {
  it('shows tokens saved since the purchase, four figures and Details', async () => {
    await render(query({}))
    const shown = text()
    expect(shown).toContain('ultrafastsince Nov 5')
    expect(shown).toContain('254ktokens saved')
    expect(facts()).toEqual([
      'time saved36 minutes',
      'ultrafast runs5',
      'steps done by neo64%',
      'fastest run3 sec',
    ])
    expect(details()).toBeTruthy()
    // The ruler, the ledger and its footer are gone.
    expect(body.querySelector('[role="img"]')).toBeNull()
    expect(shown).not.toContain('to go to')
    expect(shown).not.toContain('Pages neo accelerated')
    expect(shown).not.toContain('How we count')
  })

  it('says a milestone was passed only while it is fresh', async () => {
    await render(query({}))
    expect(text()).toContain('You just passed 250k')

    await render(query({ data: at(400_000) }))
    expect(text()).not.toContain('passed')
  })

  it('says the first run is in, with one run in the singular', async () => {
    await render(query({ data: { ...at(6_118), tasks: 1 } }))
    expect(text()).toContain('Your first ultrafast run is in')
    expect(facts()).toContain('ultrafast run1')
  })

  it('says None yet for figures neo has not measured', async () => {
    await render(
      query({
        data: {
          ...SAVINGS,
          meter: {
            ...SAVINGS.meter,
            since: SINCE,
            tokensSaved: 253_678,
            runTimes: [],
            periods: PERIODS,
            neoStepShare: undefined,
            fastestRunMs: undefined,
          },
        },
      }),
    )
    expect(facts()).toContain('steps done by neoNone yet')
    expect(facts()).toContain('fastest runNone yet')
  })

  it('lays the panel out with container queries from a wrapper', async () => {
    await render(query({}))
    const panel = body.querySelector('section[aria-label="ultrafast"]')
    // A container query can't size the element that declares the container.
    expect(panel?.parentElement?.className).toContain('@container')
    expect(panel?.className).not.toContain('@container')
  })

  it('hands a brand-new customer a task to try, with no Details yet', async () => {
    await render(
      query({
        data: {
          ...SAVINGS,
          tasks: 0,
          totals: { durationMs: 0, pagesRead: 0, tokensSaved: 0 },
          meter: {
            since: SINCE,
            tokensSaved: 0,
            runTimes: [],
            periods: PERIODS,
          },
        },
      }),
    )
    const shown = text()
    expect(shown).toContain('Let neo take the clicks.')
    expect(shown).toContain('Your numbers show up here after the first run.')
    expect(shown).toContain('Paste it into Claude Code, Codex or Cursor.')
    expect(shown).not.toContain('since Nov 5')
    expect(shown).not.toContain('tokens saved')
    expect(details()).toBeUndefined()
  })

  it('shows a loading card, then the shipped error with Try again', async () => {
    await render(query({ isPending: true, data: undefined }))
    expect(
      body
        .querySelector('section[aria-busy="true"]')
        ?.getAttribute('aria-label'),
    ).toBe('Loading your ultrafast savings')

    let refetched = false
    await render(
      query({
        isError: true,
        data: undefined,
        refetch: async () => {
          refetched = true
          return {} as never
        },
      }),
    )
    expect(body.querySelector('[role="alert"]')?.textContent).toBe(
      "Your ultrafast numbers didn't load.",
    )
    const retry = [...body.querySelectorAll('button')].find(
      (button) => button.textContent === 'Try again',
    )
    await act(async () => {
      retry?.dispatchEvent(
        new globalThis.window.Event('click', { bubbles: true }),
      )
    })
    expect(refetched).toBe(true)
  })

  it('shows the same tokens saved figure as the ultrafast page', async () => {
    const data: PlanSavings = {
      ...at(3_240_000),
      totals: { ...SAVINGS.totals, tokensSaved: 9_000 },
    }
    await act(async () =>
      root.render(
        <MemoryRouter>
          <div id="card">
            <UltrafastSummary savings={query({ data })} />
          </div>
          <div id="page">
            <SavingsHero savings={query({ data })} onShowMe={() => {}} />
          </div>
        </MemoryRouter>,
      ),
    )
    const figure = (id: string) =>
      (body.querySelector(id)?.textContent ?? '').match(
        /([\d.]+[kM]?)tokens saved/,
      )?.[1]
    expect(figure('#card')).toBe('3.2M')
    expect(figure('#page')).toBe(figure('#card'))
  })
})

describe('opening the ultrafast details', () => {
  it('tests that clicking Details counts the details being opened once', async () => {
    captured.length = 0
    await render(query({}))
    const button = details()
    if (!button) throw new Error('no Details button')
    await act(async () => {
      button.dispatchEvent(new Event('click', { bubbles: true }))
    })

    expect(
      captured.filter((event) => event === 'ultrafast_details_opened'),
    ).toEqual(['ultrafast_details_opened'])
  })
})
