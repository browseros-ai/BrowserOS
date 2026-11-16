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

const { EnergyMeter } = await import('./EnergyMeter')
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
          <EnergyMeter savings={query(data)} stats={STATS} />
        </MemoryRouter>
      </QueryClientProvider>,
    ),
  )
}

describe('the ultrafast Details', () => {
  it('opens from the meter with the since numbers', async () => {
    await renderMeter(
      savings({ since: SINCE, last30Days: SINCE, last7Days: WEEK }),
    )
    await openDetails()
    const shown = text(dialog())
    expect(dialog()?.getAttribute('aria-label')).toBe('Your savings')
    expect(tab('Since you got ultrafast')?.getAttribute('aria-selected')).toBe(
      'true',
    )
    expect(shown).toContain('326ktokens saved')
    expect(shown).toContain('Where neo helped')
    expect(shown).toContain('flipkart.com2 runs')
    expect(shown).toContain('Pages neo accelerated38')
    expect(shown).toContain('52 minutesof your time saved')
    expect(shown).toContain('14tasks finished')
    expect(shown).toContain('5ultrafast runs')
    expect(shown).toContain('1 of 2site tasks neo finished on its own')
    expect(shown).toContain('64%of the steps, done by neo')
    expect(shown).toContain('16 secaverage run, 3 sec fastest')
    expect(shown).toContain('ultrafast runs since')
    // Not the free plan's dialog.
    expect(shown).not.toContain('steps taken')
  })

  it('switches every number with the tab', async () => {
    await renderMeter(
      savings({ since: SINCE, last30Days: SINCE, last7Days: WEEK }),
    )
    await openDetails()
    await click(tab('7 days'))
    const shown = text(dialog())
    expect(tab('7 days')?.getAttribute('aria-selected')).toBe('true')
    expect(shown).toContain('109ktokens saved')
    expect(shown).toContain('4ultrafast runs')
    expect(shown).toContain('24 minutesof your time saved')
    expect(shown).toContain('Pages neo accelerated11')
    expect(shown).toContain('ultrafast runs, day by day')
    expect(shown).toContain('In the last 7 days, tokens saved adds up')

    await click(tab('30 days'))
    expect(text(dialog())).toContain('ultrafast runs, week by week')
    expect(text(dialog())).toContain(
      'The outlined week is when you got ultrafast',
    )
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
    expect(shown).toContain('No ultrafast runs in the last 7 days')
    expect(shown).toContain('Switch to "Since you got ultrafast"')
    expect(shown).toContain('No ultrafast runs in this period.')
    expect(shown).toContain('2tasks finished')
    expect(shown).not.toContain('ultrafast runs, day by day')
  })

  it('welcomes a customer who got ultrafast today', async () => {
    const fresh = period({
      tokensSaved: 1_200,
      timeSavedMs: 30_000,
      tasksFinished: 1,
    })
    await renderMeter(
      savings({ since: fresh, last30Days: fresh, last7Days: fresh }, 0),
    )
    await openDetails()
    const shown = text(dialog())
    expect(shown).toContain('Your first ultrafast run will show here')
    expect(shown).toContain(
      'Sites show up here after your first ultrafast run.',
    )
    expect(shown).not.toContain('Switch to')
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
