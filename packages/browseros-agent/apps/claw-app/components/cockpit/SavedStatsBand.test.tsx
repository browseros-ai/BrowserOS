import { afterEach, beforeEach, describe, expect, it } from 'bun:test'
import { parseHTML } from 'linkedom'
import { act } from 'react'
import type { Root } from 'react-dom/client'
import type { CockpitStats, CockpitStatsWindow } from './SavedStatsBand'

const globalDescriptors = new Map(
  [
    'window',
    'document',
    'navigator',
    'Element',
    'HTMLElement',
    'Node',
    'Event',
    'getComputedStyle',
    'requestAnimationFrame',
    'cancelAnimationFrame',
  ].map((name) => [name, Object.getOwnPropertyDescriptor(globalThis, name)]),
)

let root: Root
let container: HTMLElement
let restoreFocus: (() => void) | undefined
let SavedStatsBand: typeof import('./SavedStatsBand').SavedStatsBand

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
      browserClawTokenEstimate: 0,
      screenshotFirstTokenEstimate: 0,
      rawTokenSavingsEstimate: 0,
      humanTimeSavedMs: 0,
      sessionCount: 0,
      toolCallCount: 0,
    }),
    ...over,
  }
}

beforeEach(async () => {
  const dom = parseHTML(
    '<!doctype html><html><body><div id="root"></div></body></html>',
  )
  let nextFrameId = 0
  const cancelledFrames = new Set<number>()
  const globals = {
    window: dom.window,
    document: dom.document,
    navigator: dom.window.navigator,
    Element: dom.window.Element,
    HTMLElement: dom.window.HTMLElement,
    Node: dom.window.Node,
    Event: dom.window.Event,
    getComputedStyle: () => ({
      direction: 'ltr',
      getPropertyValue: () => '',
    }),
    requestAnimationFrame: (callback: FrameRequestCallback) => {
      const frameId = ++nextFrameId
      queueMicrotask(() => {
        if (!cancelledFrames.has(frameId)) callback(performance.now())
      })
      return frameId
    },
    cancelAnimationFrame: (frameId: number) => {
      cancelledFrames.add(frameId)
    },
  }
  for (const [name, value] of Object.entries(globals)) {
    Object.defineProperty(globalThis, name, {
      configurable: true,
      writable: true,
      value,
    })
  }

  // linkedom does not track activeElement or dispatch focus events;
  // Base UI's roving focus needs both.
  let activeElement: Element | null = null
  Object.defineProperty(dom.document, 'activeElement', {
    configurable: true,
    get: () => activeElement,
  })
  const focusDescriptor = Object.getOwnPropertyDescriptor(
    dom.window.HTMLElement.prototype,
    'focus',
  )
  Object.defineProperty(dom.window.HTMLElement.prototype, 'focus', {
    configurable: true,
    value(this: HTMLElement) {
      if (activeElement === this) return
      activeElement?.dispatchEvent(
        new dom.window.Event('focusout', { bubbles: true }),
      )
      activeElement = this
      this.dispatchEvent(new dom.window.Event('focusin', { bubbles: true }))
    },
  })
  restoreFocus = () => {
    if (focusDescriptor) {
      Object.defineProperty(
        dom.window.HTMLElement.prototype,
        'focus',
        focusDescriptor,
      )
    } else {
      Reflect.deleteProperty(dom.window.HTMLElement.prototype, 'focus')
    }
  }
  Object.defineProperty(globalThis, 'IS_REACT_ACT_ENVIRONMENT', {
    configurable: true,
    writable: true,
    value: true,
  })
  container = dom.document.getElementById('root') as unknown as HTMLElement
  const { createRoot } = await import('react-dom/client')
  // Base UI selects its layout-effect implementation at import time,
  // so load it only after installing DOM globals.
  const savedStatsModule = await import('./SavedStatsBand')
  SavedStatsBand = savedStatsModule.SavedStatsBand
  root = createRoot(container)
})

afterEach(async () => {
  await act(async () => root.unmount())
  restoreFocus?.()
  restoreFocus = undefined
  for (const [name, descriptor] of globalDescriptors) {
    if (descriptor) Object.defineProperty(globalThis, name, descriptor)
    else Reflect.deleteProperty(globalThis, name)
  }
  Reflect.deleteProperty(globalThis, 'IS_REACT_ACT_ENVIRONMENT')
})

async function render(value: CockpitStats = stats()): Promise<void> {
  await act(async () => root.render(<SavedStatsBand stats={value} />))
}

function tab(label: string): HTMLElement {
  const match = [...container.querySelectorAll('[role="tab"]')].find(
    (candidate) => candidate.textContent === label,
  )
  if (!match) throw new Error(`${label} tab missing`)
  return match as HTMLElement
}

async function selectTab(label: string): Promise<void> {
  await act(async () => {
    tab(label).dispatchEvent(new window.Event('click', { bubbles: true }))
  })
}

async function pressKey(target: HTMLElement, key: string): Promise<void> {
  await act(async () => {
    target.focus()
  })
  await act(async () => {
    const event = new window.Event('keydown', {
      bubbles: true,
      cancelable: true,
    })
    Object.defineProperties(event, {
      altKey: { value: false },
      ctrlKey: { value: false },
      getModifierState: { value: () => false },
      key: { value: key },
      metaKey: { value: false },
      shiftKey: { value: false },
    })
    target.dispatchEvent(event)
    await Promise.resolve()
  })
}

function displayedNumbers(): string[] {
  return [...container.querySelectorAll('[data-stat]')].map(
    (element) => element.textContent ?? '',
  )
}

describe('SavedStatsBand', () => {
  it('switches with arrow keys and click, including a zero recent window', async () => {
    await render()

    const allTime = tab('All time')
    expect(allTime.getAttribute('aria-selected')).toBe('true')
    expect(tab('30 days').getAttribute('aria-selected')).toBe('false')
    expect(tab('7 days').getAttribute('aria-selected')).toBe('false')
    expect(container.querySelector('[role="tablist"]')).not.toBeNull()
    expect(container.querySelector('[role="tabpanel"]')).not.toBeNull()
    expect(
      [...container.querySelectorAll('[role="tab"]')].map((item) =>
        item.getAttribute('tabindex'),
      ),
    ).toEqual(['0', '-1', '-1'])

    const allValues = displayedNumbers()
    await pressKey(allTime, 'ArrowRight')
    const monthValues = displayedNumbers()
    expect(document.activeElement?.textContent).toBe('30 days')
    expect(tab('30 days').getAttribute('aria-selected')).toBe('true')
    expect(
      monthValues.every((value, index) => value !== allValues[index]),
    ).toBe(true)

    await selectTab('7 days')
    const weekValues = displayedNumbers()
    expect(tab('7 days').getAttribute('aria-selected')).toBe('true')
    expect(
      container.querySelector('[data-stat="tokens-saved"]')?.textContent,
    ).toBe('0')
    expect(container.querySelector('[data-stat="sessions"]')?.textContent).toBe(
      '0',
    )
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
      container.querySelector('[data-used-fill]')?.getAttribute('style'),
    ).toContain('width:0%')

    await selectTab('30 days')
    expect(
      container.querySelector('[data-stat="tokens-saved"]')?.textContent,
    ).toBe('0')
    expect(
      container.querySelector('[data-stat="percentage"]')?.textContent,
    ).toBe('0%')
    expect(
      container.querySelector('[data-used-fill]')?.getAttribute('style'),
    ).toContain('width:100%')

    await selectTab('7 days')
    expect(
      container.querySelector('[data-stat="percentage"]')?.textContent,
    ).toBe('100%')
    expect(
      container.querySelector('[data-used-fill]')?.getAttribute('style'),
    ).toContain('width:50%')
    expect(value.allTime.rawTokenSavingsEstimate).toBe(-50)
  })

  it('keeps both comparison labels outside the bounded bar at every ratio', async () => {
    // The original overlap bug: both token labels were absolutely positioned
    // inside the same overflow-hidden track and painted through each other in
    // the mid-range. The fix moves them into a legend row that is a sibling of
    // the bar, so no ratio can make them collide or clip. Assert the structural
    // invariant that guarantees it — the track carries no text labels — across
    // a low, mid, and full used ratio.
    const value = stats({
      allTime: statsWindow({
        browserClawTokenEstimate: 63_200,
        screenshotFirstTokenEstimate: 112_300,
        rawTokenSavingsEstimate: 49_100,
      }),
      last30Days: statsWindow({
        browserClawTokenEstimate: 2_000,
        screenshotFirstTokenEstimate: 200_000,
        rawTokenSavingsEstimate: 198_000,
      }),
      last7Days: statsWindow({
        browserClawTokenEstimate: 500_000,
        screenshotFirstTokenEstimate: 500_000,
        rawTokenSavingsEstimate: 0,
      }),
    })
    await render(value)

    for (const label of ['All time', '30 days', '7 days']) {
      await selectTab(label)
      const track = container.querySelector('[data-budget-track]')
      expect(track).not.toBeNull()
      // Root-cause guard: nothing textual lives inside the bounded bar.
      expect(track?.querySelector('[data-stat]')).toBeNull()
      expect(track?.textContent?.trim()).toBe('')
      // Both numbers still render, now in the legend, and no legend label is
      // absolutely positioned (the property that let them overlap).
      const browserclaw = container.querySelector(
        '[data-stat="browserclaw-tokens"]',
      )
      const comparison = container.querySelector(
        '[data-stat="comparison-tokens"]',
      )
      expect(browserclaw?.textContent?.trim().length).toBeGreaterThan(0)
      expect(comparison?.textContent?.trim().length).toBeGreaterThan(0)
      expect(browserclaw?.closest('[class*="absolute"]')).toBeNull()
      expect(comparison?.closest('[class*="absolute"]')).toBeNull()
    }
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
