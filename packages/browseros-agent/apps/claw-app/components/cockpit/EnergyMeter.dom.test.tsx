/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { afterEach, beforeEach, describe, expect, it } from 'bun:test'
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

const { EnergyMeter } = await import('./EnergyMeter')
const { scalePosition, scaleSpan } = await import('./energy-meter.helpers')
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
  since: period({ tokensSaved: 253_678, runs: 5, pagesRead: 38 }),
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
let storage: Map<string, string>
let reducedMotion: boolean

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
    root.render(<EnergyMeter savings={savings} stats={undefined} />),
  )
}

function text(): string {
  return (body.textContent ?? '').replace(/\s+/g, ' ')
}

beforeEach(async () => {
  storage = new Map()
  reducedMotion = true
  const dom = parseHTML(
    '<!doctype html><html><body><div id="root"></div></body></html>',
  )
  Object.defineProperty(dom.window, 'matchMedia', {
    configurable: true,
    value: () => ({ matches: reducedMotion }),
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

function needle(): string {
  return (
    body.querySelector('[data-needle]')?.getAttribute('style') ?? ''
  ).replace(/\s+/g, '')
}

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

describe('the ultrafast meter', () => {
  it('reads the total off the scale, with the next milestone and the ledger', async () => {
    await render(query({}))
    const shown = text()
    expect(shown).toContain('ultrafastsince Nov 5')
    expect(shown).toContain('254ktokens saved')
    expect(shown).toContain('246k to go to 500k')
    expect(shown).toContain('You just passed 250k')
    expect(shown).toContain('Pages neo accelerated38')
    expect(shown).toContain('ultrafast runs5')
    expect(shown).toContain('Steps done by neo64%')
    expect(shown).toContain('Fastest run3 sec')
    expect(body.querySelector('[role="img"]')?.getAttribute('aria-label')).toBe(
      '254k tokens saved, 246k to go to the next milestone',
    )
    expect(
      body.querySelector('button[aria-label="How tokens saved is counted"]'),
    ).toBeTruthy()
    expect(
      [...body.querySelectorAll('button')].some((button) =>
        button.textContent?.includes('Details'),
      ),
    ).toBe(true)
  })

  it('rules the stretch from the last milestone to the next, with the earlier ones in a trail', async () => {
    await render(query({}))
    const states = [...body.querySelectorAll('[data-mark]')].map((mark) =>
      mark.getAttribute('data-mark'),
    )
    expect(states).toEqual(['passed', 'next'])
    const ruler = body.querySelector('[role="img"]')?.textContent ?? ''
    expect(ruler.startsWith('250k')).toBe(true)
    expect(ruler.endsWith('500k')).toBe(true)
    // 10k, 25k, 50k and 100k were passed before this stretch.
    expect(text()).toContain('4 passed')
  })

  it('starts a first stretch at 0 with no trail', async () => {
    await render(query({ data: at(6_118) }))
    expect(
      [...body.querySelectorAll('[data-mark]')].map((mark) =>
        mark.getAttribute('data-mark'),
      ),
    ).toEqual(['next'])
    expect(text()).not.toMatch(/\d passed/)
    expect(text()).toContain('3.9k to go to 10k')
  })

  it('moves the needle once from the last reading on this computer', async () => {
    reducedMotion = false
    storage.set('ultrafastMeterReading:v1', '251000')
    await render(query({}))
    const from = needle()
    await act(async () => new Promise((resolve) => setTimeout(resolve, 40)))
    const to = needle()
    expect(from).not.toBe(to)
    expect(to).toContain(
      `translateX(${scalePosition(253_678, scaleSpan(253_678)) * 100}%)`,
    )
    expect(from).toContain(
      `translateX(${scalePosition(251_000, scaleSpan(253_678)) * 100}%)`,
    )
    expect(storage.get('ultrafastMeterReading:v1')).toBe('253678')
  })

  it('sets the needle straight down for readers who prefer less motion', async () => {
    storage.set('ultrafastMeterReading:v1', '200000')
    await render(query({}))
    const placed = needle()
    await act(async () => new Promise((resolve) => setTimeout(resolve, 40)))
    expect(needle()).toBe(placed)
  })

  it('says a milestone crossed while Home is open', async () => {
    await render(query({ data: at(80_000) }))
    expect(text()).toContain('20k to go to 100k')
    expect(text()).toContain('Passed 50k')

    // The next poll brings the total past 100k without leaving Home.
    await render(query({ data: at(101_500) }))
    expect(text()).toContain('You just passed 100k')
    expect(text()).toContain('149k to go to 250k')
  })

  it('lays the panel out with container queries from a wrapper', async () => {
    await render(query({}))
    const panel = body.querySelector('section[aria-label="ultrafast"]')
    // A container query can't size the element that declares the container.
    expect(panel?.parentElement?.className).toContain('@container')
    expect(panel?.className).not.toContain('@container')
  })

  it('invites a brand-new customer to a first task', async () => {
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
    expect(shown).toContain('0tokens saved')
    expect(shown).toContain('First milestone: 10k.')
    expect(shown).not.toContain('Pages neo accelerated')
  })

  it('shows the same tokens saved figure as the ultrafast page', async () => {
    const data: PlanSavings = {
      ...at(3_240_000),
      totals: { ...SAVINGS.totals, tokensSaved: 9_000 },
    }
    await act(async () =>
      root.render(
        <MemoryRouter>
          <div id="meter">
            <EnergyMeter savings={query({ data })} stats={undefined} />
          </div>
          <div id="page">
            <SavingsHero savings={query({ data })} onShowMe={() => {}} />
          </div>
        </MemoryRouter>,
      ),
    )
    // The meter reads its figure off the scale; the page sets it beside "tokens saved".
    const meter = body
      .querySelector('#meter [role="img"]')
      ?.getAttribute('aria-label')
      ?.split(' ')[0]
    const page = (body.querySelector('#page')?.textContent ?? '').match(
      /([\d.]+[kM]?)tokens saved/,
    )?.[1]
    expect(meter).toBe('3.2M')
    expect(page).toBe(meter)
  })

  it('keeps the panel shape while loading and offers a retry when loading failed', async () => {
    await render(query({ data: undefined, isPending: true }))
    const loading = body.querySelector(
      'section[aria-label="Loading your ultrafast savings"]',
    )
    expect(loading?.getAttribute('aria-busy')).toBe('true')
    expect(text()).toContain('Counting what neo saved')

    let retried = false
    await render(
      query({
        data: undefined,
        isError: true,
        refetch: async () => {
          retried = true
          return {} as Awaited<
            ReturnType<UseQueryResult<PlanSavings>['refetch']>
          >
        },
      }),
    )
    expect(text()).toContain("Your ultrafast numbers didn't load.")
    const retry = [...body.querySelectorAll('button')].find((button) =>
      button.textContent?.includes('Try again'),
    )
    await act(async () => {
      retry?.dispatchEvent(
        new globalThis.window.Event('click', { bubbles: true }),
      )
    })
    expect(retried).toBe(true)
  })
})
