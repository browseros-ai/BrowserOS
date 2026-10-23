/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { afterEach, beforeEach, describe, expect, it, mock } from 'bun:test'
import { parseHTML } from 'linkedom'
import { act } from 'react'
import type { Root } from 'react-dom/client'
import * as realEvents from '@/modules/analytics/events'

const tracked: Array<[string, unknown]> = []
mock.module('@/modules/analytics/events', () => ({
  ...realEvents,
  track: (event: string, properties?: unknown) =>
    tracked.push([event, properties]),
}))

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
] as const
const globalDescriptors = new Map(
  globalNames.map((name) => [
    name,
    Object.getOwnPropertyDescriptor(globalThis, name),
  ]),
)

const { OnboardingDemo } = await import('./OnboardingDemo')

let root: Root
let container: HTMLElement
let reducedMotion = true

function define(name: string, value: unknown) {
  Object.defineProperty(globalThis, name, {
    configurable: true,
    writable: true,
    value,
  })
}

beforeEach(async () => {
  tracked.length = 0
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
  define('IS_REACT_ACT_ENVIRONMENT', true)
  container = dom.document.getElementById('root') as unknown as HTMLElement
  const { createRoot } = await import('react-dom/client')
  root = createRoot(container)
  await act(async () => root.render(<OnboardingDemo />))
})

afterEach(async () => {
  await act(async () => root.unmount())
  for (const [name, descriptor] of globalDescriptors) {
    if (descriptor) Object.defineProperty(globalThis, name, descriptor)
    else Reflect.deleteProperty(globalThis, name)
  }
  Reflect.deleteProperty(globalThis, 'IS_REACT_ACT_ENVIRONMENT')
  reducedMotion = true
})

function tile(app: 'Terminal' | 'Claude' | 'ChatGPT') {
  return container.querySelector(
    `button[aria-label="Show it in ${app}"]`,
  ) as HTMLButtonElement
}

function description() {
  return container.querySelector('.sr-only')?.textContent ?? ''
}

async function click(target: HTMLElement) {
  await act(async () => {
    target.dispatchEvent(new window.Event('click', { bubbles: true }))
  })
}

describe('OnboardingDemo dock', () => {
  it('makes Terminal, Claude and ChatGPT named buttons, and nothing else', () => {
    const buttons = [...container.querySelectorAll('button[aria-pressed]')]
    expect(buttons.map((button) => button.getAttribute('aria-label'))).toEqual([
      'Show it in Terminal',
      'Show it in Claude',
      'Show it in ChatGPT',
    ])
    // Native buttons, so Enter and Space activate them from the keyboard.
    for (const button of buttons) {
      expect(button.tagName).toBe('BUTTON')
      expect(button.getAttribute('type')).toBe('button')
    }
  })

  it('plays Claude by default', () => {
    expect(tile('Claude').getAttribute('aria-pressed')).toBe('true')
    expect(description()).toContain('the Claude app, switch it to Code')
  })

  it('clicking a tile switches the flow and marks that tile', async () => {
    await click(tile('Terminal'))
    expect(tile('Terminal').getAttribute('aria-pressed')).toBe('true')
    expect(tile('Claude').getAttribute('aria-pressed')).toBe('false')
    expect(description()).toContain('open Terminal, start Claude Code')
    expect(container.textContent).toContain('claude — ~')

    await click(tile('ChatGPT'))
    expect(tile('ChatGPT').getAttribute('aria-pressed')).toBe('true')
    expect(description()).toContain('switch it to Work')
    expect(tracked).toContainEqual([
      'onboarding_link_clicked',
      { link: 'demo-chatgpt' },
    ])
  })

  it('with reduced motion, each flow shows its Done frame and stays paused', async () => {
    for (const app of ['Terminal', 'ChatGPT', 'Claude'] as const) {
      await click(tile(app))
      expect(container.textContent).toContain(
        'Cheapest is $142, nonstop, Fri 7:05 am.',
      )
      expect(
        container.querySelector('button[aria-label="Play the demo"]'),
      ).not.toBeNull()
    }
  })

  it('without reduced motion, a click plays the flow from the start', async () => {
    reducedMotion = false
    await act(async () => root.unmount())
    const { createRoot } = await import('react-dom/client')
    root = createRoot(container)
    await act(async () => root.render(<OnboardingDemo />))
    await act(async () => {
      container
        .querySelector('button[aria-label="Pause the demo"]')
        ?.dispatchEvent(new window.Event('click', { bubbles: true }))
    })
    await click(tile('Terminal'))
    expect(
      container.querySelector('button[aria-label="Pause the demo"]'),
    ).not.toBeNull()
    expect(tile('Terminal').getAttribute('aria-pressed')).toBe('true')
  })
})
