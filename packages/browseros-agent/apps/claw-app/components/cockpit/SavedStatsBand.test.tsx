import { afterEach, beforeEach, describe, expect, it } from 'bun:test'
import { parseHTML } from 'linkedom'
import { act } from 'react'
import type { Root } from 'react-dom/client'
import {
  type CockpitStats,
  type CockpitStatsWindow,
  SavedStatsBand,
} from './SavedStatsBand'

const globalDescriptors = new Map(
  ['window', 'document', 'navigator', 'HTMLElement', 'Node', 'Event'].map(
    (name) => [name, Object.getOwnPropertyDescriptor(globalThis, name)],
  ),
)

let root: Root
let container: HTMLElement

function statsWindow(
  over: Partial<CockpitStatsWindow> = {},
): CockpitStatsWindow {
  return {
    browserClawTokenEstimate: 100,
    screenshotFirstTokenEstimate: 1_000,
    rawTokenSavingsEstimate: 900,
    humanTimeSavedMs: 60 * 60 * 1_000,
    sessionCount: 1,
    toolCallCount: 10,
    ...over,
  }
}

function stats(over: Partial<CockpitStats> = {}): CockpitStats {
  return {
    hasMeasuredStats: true,
    allTime: statsWindow(),
    last30Days: statsWindow({
      browserClawTokenEstimate: 200,
      screenshotFirstTokenEstimate: 1_200,
      rawTokenSavingsEstimate: 800,
      humanTimeSavedMs: 2 * 60 * 60 * 1_000,
      sessionCount: 2,
      toolCallCount: 20,
    }),
    last7Days: statsWindow({
      browserClawTokenEstimate: 300,
      screenshotFirstTokenEstimate: 1_500,
      rawTokenSavingsEstimate: 600,
      humanTimeSavedMs: 3 * 60 * 60 * 1_000,
      sessionCount: 3,
      toolCallCount: 30,
    }),
    ...over,
  }
}

beforeEach(async () => {
  const dom = parseHTML(
    '<!doctype html><html><body><div id="root"></div></body></html>',
  )
  const globals = {
    window: dom.window,
    document: dom.document,
    navigator: dom.window.navigator,
    HTMLElement: dom.window.HTMLElement,
    Node: dom.window.Node,
    Event: dom.window.Event,
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

async function render(value: CockpitStats = stats()): Promise<void> {
  await act(async () => root.render(<SavedStatsBand stats={value} />))
}

function tab(label: string): Element {
  const match = [...container.querySelectorAll('[role="tab"]')].find(
    (candidate) => candidate.textContent === label,
  )
  if (!match) throw new Error(`${label} tab missing`)
  return match
}

async function selectTab(label: string): Promise<void> {
  await act(async () => {
    tab(label).dispatchEvent(new window.Event('click', { bubbles: true }))
  })
}

function displayedNumbers(): string[] {
  return [...container.querySelectorAll('[data-stat]')].map(
    (element) => element.textContent ?? '',
  )
}

describe('SavedStatsBand', () => {
  it('selects All time by default and switches every displayed number', async () => {
    await render()

    const allTime = tab('All time')
    expect(allTime.getAttribute('aria-selected')).toBe('true')
    expect(tab('30 days').getAttribute('aria-selected')).toBe('false')
    expect(tab('7 days').getAttribute('aria-selected')).toBe('false')
    expect(container.querySelector('[role="tablist"]')).not.toBeNull()
    expect(container.querySelector('[role="tabpanel"]')).not.toBeNull()

    const allValues = displayedNumbers()
    await selectTab('30 days')
    const monthValues = displayedNumbers()
    expect(tab('30 days').getAttribute('aria-selected')).toBe('true')
    expect(
      monthValues.every((value, index) => value !== allValues[index]),
    ).toBe(true)

    await selectTab('7 days')
    const weekValues = displayedNumbers()
    expect(tab('7 days').getAttribute('aria-selected')).toBe('true')
    expect(
      weekValues.every((value, index) => value !== monthValues[index]),
    ).toBe(true)
  })

  it('formats zero, thousands, millions, minutes, and hours deterministically', async () => {
    await render(
      stats({
        allTime: statsWindow({
          browserClawTokenEstimate: 0,
          screenshotFirstTokenEstimate: 0,
          rawTokenSavingsEstimate: 0,
          humanTimeSavedMs: 5 * 60 * 1_000,
          sessionCount: 0,
          toolCallCount: 0,
        }),
        last30Days: statsWindow({
          browserClawTokenEstimate: 7_600,
          screenshotFirstTokenEstimate: 20_000,
          rawTokenSavingsEstimate: 12_400,
          humanTimeSavedMs: 45 * 60 * 1_000,
          sessionCount: 12,
          toolCallCount: 1_234,
        }),
        last7Days: statsWindow({
          browserClawTokenEstimate: 300_000,
          screenshotFirstTokenEstimate: 1_500_000,
          rawTokenSavingsEstimate: 1_200_000,
          humanTimeSavedMs: (4 * 60 + 5) * 60 * 1_000,
          sessionCount: 1_200,
          toolCallCount: 12_400,
        }),
      }),
    )

    expect(
      container.querySelector('[data-stat="tokens-saved"]')?.textContent,
    ).toBe('0')
    expect(
      container.querySelector('[data-stat="human-time"]')?.textContent,
    ).toBe('5m')

    await selectTab('30 days')
    expect(
      container.querySelector('[data-stat="tokens-saved"]')?.textContent,
    ).toBe('12.4K')
    expect(
      container.querySelector('[data-stat="human-time"]')?.textContent,
    ).toBe('45m')
    expect(
      container.querySelector('[data-stat="tool-calls"]')?.textContent,
    ).toBe('1,234')

    await selectTab('7 days')
    expect(
      container.querySelector('[data-stat="tokens-saved"]')?.textContent,
    ).toBe('1.2M')
    expect(
      container.querySelector('[data-stat="human-time"]')?.textContent,
    ).toBe('4h 05m')
    expect(container.querySelector('[data-stat="sessions"]')?.textContent).toBe(
      '1,200',
    )
  })

  it('clamps visible savings, percentage, and marker position without mutating input', async () => {
    const value = stats({
      allTime: statsWindow({
        browserClawTokenEstimate: 100,
        screenshotFirstTokenEstimate: 0,
        rawTokenSavingsEstimate: -50,
      }),
      last30Days: statsWindow({
        browserClawTokenEstimate: 200,
        screenshotFirstTokenEstimate: 100,
        rawTokenSavingsEstimate: -100,
      }),
      last7Days: statsWindow({
        browserClawTokenEstimate: 50,
        screenshotFirstTokenEstimate: 100,
        rawTokenSavingsEstimate: 200,
      }),
    })
    await render(value)

    expect(
      container.querySelector('[data-stat="tokens-saved"]')?.textContent,
    ).toBe('0')
    expect(
      container.querySelector('[data-stat="percentage"]')?.textContent,
    ).toBe('0%')
    expect(
      container.querySelector('[data-used-marker]')?.getAttribute('style'),
    ).toContain('left:0%')

    await selectTab('30 days')
    expect(
      container.querySelector('[data-stat="tokens-saved"]')?.textContent,
    ).toBe('0')
    expect(
      container.querySelector('[data-stat="percentage"]')?.textContent,
    ).toBe('0%')
    expect(
      container.querySelector('[data-used-marker]')?.getAttribute('style'),
    ).toContain('left:100%')

    await selectTab('7 days')
    expect(
      container.querySelector('[data-stat="percentage"]')?.textContent,
    ).toBe('100%')
    expect(
      container.querySelector('[data-used-marker]')?.getAttribute('style'),
    ).toContain('left:50%')
    expect(value.allTime.rawTokenSavingsEstimate).toBe(-50)
  })

  it('frames BrowserClaw against a screenshot-first agent', async () => {
    await render()

    expect(container.textContent).toContain(
      'a screenshot-first agent would spend',
    )
    expect(container.textContent).toContain(
      'compact DOM & tool responses instead of a screenshot per call',
    )
    expect(container.textContent).not.toContain('DOM-dump agent')
    expect(container.textContent).not.toContain(
      'scaled screenshots instead of full-page DOM dumps',
    )
  })

  it('stacks at narrow widths and disables the decorative loop for reduced motion', async () => {
    await render()

    const card = container.querySelector('[data-saved-stats-card]')
    const track = container.querySelector('[data-budget-track]')
    const ping = container.querySelector('[data-used-marker-ping]')
    expect(card?.getAttribute('class')).toContain('flex-col')
    expect(card?.getAttribute('class')).toContain('md:flex-row')
    expect(track?.getAttribute('class')).toContain('overflow-hidden')
    expect(ping?.getAttribute('class')).toContain('animate-ping')
    expect(ping?.getAttribute('class')).toContain('motion-reduce:animate-none')
    expect(container.querySelector('[data-stat="tokens-saved"]')).not.toBeNull()
  })
})
