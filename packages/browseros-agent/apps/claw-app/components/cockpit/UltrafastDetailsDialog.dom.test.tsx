/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { afterEach, beforeEach, describe, expect, it } from 'bun:test'
import type {
  CockpitStats,
  CockpitStatsWindow,
  PlanSavings,
  PlanSavingsPeriod,
} from '@browseros/claw-api'
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

const { UltrafastSummary } = await import('./UltrafastSummary')
const { SavingsCard } = await import('./SavingsCard')
const { MemoryRouter } = await import('react-router')
const { QueryClient, QueryClientProvider } = await import(
  '@tanstack/react-query'
)

const DAY = 24 * 60 * 60 * 1000
const NOW = Date.now()

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

const SINCE = period({
  tokensSaved: 326_000,
  runs: 5,
  pagesRead: 38,
  durationMs: 80_000,
  fastestRunMs: 3_000,
  neoStepShare: 64,
  siteTasks: 2,
  siteTasksFinished: 1,
  topSites: [
    { site: 'flipkart.com', runs: 2 },
    { site: 'google.com', runs: 1 },
    { site: 'amazon.in', runs: 1 },
  ],
  timeSavedMs: 52 * 60_000,
  tasksFinished: 14,
  runTimes: [NOW, NOW - DAY, NOW - 4 * DAY, NOW - 5 * DAY, NOW - 10 * DAY],
})

const WEEK = period({
  tokensSaved: 109_000,
  runs: 4,
  pagesRead: 11,
  durationMs: 32_000,
  fastestRunMs: 3_000,
  neoStepShare: 64,
  siteTasks: 2,
  siteTasksFinished: 1,
  topSites: [{ site: 'google.com', runs: 1 }],
  timeSavedMs: 24 * 60_000,
  tasksFinished: 7,
  runTimes: [NOW, NOW - DAY, NOW - 4 * DAY, NOW - 5 * DAY],
})

function savings(
  periods: PlanSavings['meter'] extends infer M
    ? M extends { periods: infer P }
      ? P
      : never
    : never,
  tasks = 5,
): PlanSavings {
  return {
    totals: { durationMs: 80_000, pagesRead: 38, tokensSaved: 254_000 },
    tasks,
    latest: [],
    meter: {
      since: NOW - 12 * DAY,
      tokensSaved: periods.since.tokensSaved,
      runTimes: [],
      periods,
    },
  }
}

function statsWindow(): CockpitStatsWindow {
  return {
    browserClawTokenEstimate: 1_000,
    screenshotFirstTokenEstimate: 9_000,
    rawTokenSavingsEstimate: 8_000,
    humanTimeSavedMs: 3_600_000,
    sessionCount: 18,
    toolCallCount: 539,
  }
}

const STATS: CockpitStats = {
  hasMeasuredStats: true,
  allTime: statsWindow(),
  last30Days: statsWindow(),
  last7Days: statsWindow(),
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

function query(data: PlanSavings): UseQueryResult<PlanSavings> {
  return {
    data,
    isPending: false,
    isError: false,
    refetch: async () => ({}),
  } as unknown as UseQueryResult<PlanSavings>
}

function text(node: Element | null = body): string {
  return (node?.textContent ?? '').replace(/\s+/g, ' ')
}

function dialog(): Element | null {
  return body.querySelector('[role="dialog"]')
}

async function click(target: Element | null | undefined) {
  if (!target) throw new Error('nothing to click')
  await act(async () => {
    target.dispatchEvent(
      new globalThis.window.Event('click', { bubbles: true }),
    )
  })
}

async function openDetails() {
  await click(body.querySelector('button[aria-haspopup="dialog"]'))
}

function tab(label: string) {
  return [...body.querySelectorAll('[role="tab"]')].find(
    (node) => node.textContent === label,
  )
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
  const storage = new Map<string, string>()
  define('localStorage', {
    getItem: (key: string) => storage.get(key) ?? null,
    setItem: (key: string, value: string) => storage.set(key, value),
  })
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

async function renderMeter(data: PlanSavings) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false, enabled: false } },
  })
  await act(async () =>
    root.render(
      <QueryClientProvider client={client}>
        <MemoryRouter>
          <UltrafastSummary savings={query(data)} />
        </MemoryRouter>
      </QueryClientProvider>,
    ),
  )
}

const PURCHASED = new Date(NOW - 12 * DAY).toLocaleDateString('en-US', {
  month: 'short',
  day: 'numeric',
})

/** A heavy month: up to 40 runs a day since the purchase. */
const BUSY = [23, 31, 8, 37, 19, 0, 4, 28, 40, 26, 15, 11, 3]
const HEAVY = period({
  ...SINCE,
  tokensSaved: 1_850_000,
  runs: BUSY.reduce((sum, runs) => sum + runs, 0),
  runTimes: BUSY.flatMap((runs, daysAgo) =>
    Array.from({ length: runs }, () => NOW - daysAgo * DAY),
  ),
})

function strip(): Element | null {
  return dialog()?.querySelector('[data-strip]') ?? null
}

function figures(): string[] {
  return [...(dialog()?.querySelectorAll('dl > div') ?? [])].map((fact) =>
    (fact.textContent ?? '').replace(/\s+/g, ' '),
  )
}

describe('the ultrafast Details', () => {
  it('opens from the card on the since numbers: the total, six figures, the runs and how we count', async () => {
    await renderMeter(
      savings({ since: SINCE, last30Days: SINCE, last7Days: WEEK }),
    )
    await openDetails()
    const shown = text(dialog())
    expect(dialog()?.getAttribute('aria-label')).toBe('Your savings')
    expect(text(dialog()?.querySelector('h2') ?? null)).toBe(
      'ultrafast Your savings',
    )
    expect(tab('Since you got ultrafast')?.getAttribute('aria-selected')).toBe(
      'true',
    )
    expect(shown).toContain(`326ktokens saved since ${PURCHASED}`)
    expect(figures()).toEqual([
      'of your time saved52 minutes',
      'ultrafast runs5',
      'steps done by neo64%',
      'fastest run3 sec',
      'average run16 sec',
      'pages neo accelerated38',
    ])
    expect(text(dialog()?.querySelector('h3') ?? null)).toBe(
      'Runs1 of 2 site tasks finished',
    )
    expect(shown).toContain('They are measured differently')
    // Sites, the task count and the latest runs are not in this popup.
    expect(shown).not.toContain('Where neo helped')
    expect(shown).not.toContain('flipkart.com')
    expect(shown).not.toContain('tasks finished14')
    expect(shown).not.toContain('Latest ultrafast runs')
    // Not the free plan's dialog.
    expect(shown).not.toContain('steps taken')
  })

  it('switches every number and the strip with the tab', async () => {
    await renderMeter(
      savings({ since: SINCE, last30Days: SINCE, last7Days: WEEK }),
    )
    await openDetails()
    await click(tab('7 days'))
    expect(tab('7 days')?.getAttribute('aria-selected')).toBe('true')
    expect(tab('Since you got ultrafast')?.getAttribute('aria-selected')).toBe(
      'false',
    )
    expect(text(dialog())).toContain('109ktokens saved in the last 7 days')
    expect(figures()).toContain('ultrafast runs4')
    expect(figures()).toContain('of your time saved24 minutes')
    expect(figures()).toContain('pages neo accelerated11')
    expect(strip()?.querySelectorAll('[data-tip]').length).toBe(7)

    await click(tab('30 days'))
    expect(text(dialog())).toContain('tokens saved in the last 30 days')
    // One column a day, the days before the purchase on their own baseline.
    expect(strip()?.querySelectorAll('[data-tip]').length).toBe(30)
    expect(
      strip()?.querySelectorAll('[data-tip$="before ultrafast"]').length,
    ).toBe(17)
    expect(text(strip())).toContain('before ultrafast')
  })

  it('stacks one square per run while the days are light', async () => {
    await renderMeter(
      savings({ since: SINCE, last30Days: SINCE, last7Days: WEEK }),
    )
    await openDetails()
    expect(strip()?.getAttribute('data-strip')).toBe('squares')
    expect(strip()?.querySelectorAll('[data-square]').length).toBe(5)
    expect(text(strip())).toContain('One square is one ultrafast run.')
  })

  it('draws bars once a day passes eight runs, with only the busiest one counted', async () => {
    await renderMeter(
      savings({ since: HEAVY, last30Days: HEAVY, last7Days: WEEK }),
    )
    await openDetails()
    expect(strip()?.getAttribute('data-strip')).toBe('bars')
    expect(strip()?.querySelectorAll('[data-square]').length).toBe(0)
    expect(
      strip()?.querySelector('[role="img"]')?.getAttribute('aria-label'),
    ).toContain('245 ultrafast runs, the busiest day')
    expect(
      [...(strip()?.querySelectorAll('em') ?? [])].map(
        (count) => count.textContent,
      ),
    ).toEqual(['40'])
    expect(text(strip())).toContain('Runs per day. The busiest day is marked.')
    const busiest = strip()?.querySelector('[data-runs="40"]')
    expect(busiest?.getAttribute('data-tip')).toMatch(/: 40 runs$/)
  })

  it('says so when a period has no runs', async () => {
    await renderMeter(
      savings({
        since: SINCE,
        last30Days: SINCE,
        last7Days: period({
          tokensSaved: 12_000,
          timeSavedMs: 360_000,
          tasksFinished: 2,
        }),
      }),
    )
    await openDetails()
    await click(tab('7 days'))
    const shown = text(dialog())
    expect(shown).toContain('No ultrafast runs in the last 7 days.')
    expect(shown).toContain('Switch to "Since you got ultrafast"')
    expect(figures()).toEqual([])
    expect(strip()).toBeNull()
    expect(shown).toContain('They are measured differently')
  })
})

describe('free installs', () => {
  it('keep the savings dialog without the ultrafast Details', async () => {
    await act(async () => root.render(<SavingsCard stats={STATS} />))
    await openDetails()
    const shown = text(dialog())
    expect(shown).toContain('steps taken')
    expect(shown).toContain('All time')
    expect(shown).not.toContain('Where neo helped')
    expect(shown).not.toContain('Since you got ultrafast')
  })
})
