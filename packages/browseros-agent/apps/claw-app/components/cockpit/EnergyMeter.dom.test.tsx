/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { afterEach, beforeEach, describe, expect, it } from 'bun:test'
import type { PlanSavings } from '@browseros/claw-api'
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

const SAVINGS: PlanSavings = {
  totals: { durationMs: 80_000, pagesRead: 38, tokensSaved: 253_678 },
  tasks: 5,
  latest: [],
  meter: {
    since: 1,
    tokensSaved: 253_678,
    runTimes: [],
    neoStepShare: 64,
    fastestRunMs: 3_095,
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

describe('the ultrafast meter', () => {
  it('shows the total, the milestone and the four tiles', async () => {
    await render(query({}))
    const shown = text()
    expect(shown).toContain('254ktokens saved')
    expect(shown).toContain('since you got ultrafast')
    expect(shown).toContain('You passed 250k tokens saved')
    expect(shown).toContain('38pages neo accelerated')
    expect(shown).toContain('5ultrafast runs')
    expect(shown).toContain('64%of the steps, done by neo')
    expect(shown).toContain('3 secyour fastest ultrafast run')
    expect(
      body.querySelector('button[aria-label="How tokens saved is counted"]'),
    ).toBeTruthy()
  })

  it('bursts once per milestone on this computer', async () => {
    await render(query({}))
    expect(body.querySelector('[data-burst]')).toBeTruthy()
    expect(storage.get('ultrafastMilestoneSeen:v1')).toBe('250000')

    await act(async () => root.unmount())
    const { createRoot } = await import('react-dom/client')
    root = createRoot(body.querySelector('#root') as unknown as HTMLElement)
    await render(query({}))
    expect(body.querySelector('[data-burst]')).toBeNull()
    expect(text()).toContain('You passed 250k tokens saved')
  })

  it('plays the burst for a milestone crossed while Home is open', async () => {
    const at = (tokensSaved: number): PlanSavings => ({
      ...SAVINGS,
      meter: { ...SAVINGS.meter, since: 1, runTimes: [], tokensSaved },
    })
    await render(query({ data: at(61_000) }))
    expect(text()).toContain('39k to go to 100k')
    expect(body.querySelector('[data-burst]')).toBeNull()

    // The next poll brings the total past 100k without leaving Home.
    await render(query({ data: at(101_500) }))
    expect(text()).toContain('You passed 100k tokens saved')
    expect(body.querySelector('[data-burst]')).toBeTruthy()
    expect(storage.get('ultrafastMilestoneSeen:v1')).toBe('100000')
  })

  it('lays the panel out with container queries from a wrapper', async () => {
    await render(query({}))
    const panel = body.querySelector('section[aria-label="ultrafast"]')
    // A container query can't size the element that declares the container.
    expect(panel?.parentElement?.className).toContain('@container')
    expect(panel?.className).not.toContain('@container')
    expect(panel?.className).toContain('@2xl:grid-cols-[auto_1fr]')
  })

  it('counts the numbers up when motion is allowed', async () => {
    reducedMotion = false
    await render(query({}))
    expect(text()).toContain('0tokens saved')
    await act(async () => new Promise((resolve) => setTimeout(resolve, 1_800)))
    expect(text()).toContain('254ktokens saved')
  })

  it('invites a brand-new customer to a first task', async () => {
    await render(
      query({
        data: {
          ...SAVINGS,
          tasks: 0,
          totals: { durationMs: 0, pagesRead: 0, tokensSaved: 0 },
          meter: { since: 1, tokensSaved: 0, runTimes: [] },
        },
      }),
    )
    const shown = text()
    expect(shown).toContain('Let neo take the clicks.')
    expect(shown).toContain('0tokens saved')
    expect(shown).toContain('First milestone: 10k.')
    expect(shown).not.toContain('pages neo accelerated')
  })

  it('shows a placeholder while loading and a retry when loading failed', async () => {
    await render(query({ data: undefined, isPending: true }))
    expect(
      body.querySelector('[aria-label="Loading your ultrafast savings"]'),
    ).toBeTruthy()

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
