/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { afterEach, beforeEach, describe, expect, it } from 'bun:test'
import { parseHTML } from 'linkedom'
import { act } from 'react'
import type { Root } from 'react-dom/client'
import type { Harness } from '@/components/harness/harness.types'

const globalDescriptors = new Map(
  ['window', 'document', 'navigator', 'HTMLElement', 'Node', 'Event'].map(
    (name) => [name, Object.getOwnPropertyDescriptor(globalThis, name)],
  ),
)

const { ConnectedApps } = await import('./ConnectedApps')

const SEVEN: Harness[] = [
  'Claude Code',
  'Codex',
  'Cursor',
  'OpenCode',
  'Antigravity',
  'VS Code',
  'Zed',
]

let root: Root
let container: HTMLElement

beforeEach(async () => {
  const dom = parseHTML(
    '<!doctype html><html><body><div id="root"></div><p id="outside"></p></body></html>',
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
  await act(async () => root.render(<ConnectedApps harnesses={SEVEN} />))
})

afterEach(async () => {
  await act(async () => root.unmount())
  for (const [name, descriptor] of globalDescriptors) {
    if (descriptor) Object.defineProperty(globalThis, name, descriptor)
    else Reflect.deleteProperty(globalThis, name)
  }
  Reflect.deleteProperty(globalThis, 'IS_REACT_ACT_ENVIRONMENT')
})

function trigger() {
  return container.querySelector('button[aria-expanded]') as HTMLButtonElement
}

/** App names as listed, without the marks' SVG titles. */
function listedApps() {
  return [...container.querySelectorAll('li')].map((li) =>
    [...li.childNodes]
      .filter((node) => node.nodeType === 3)
      .map((node) => node.textContent)
      .join(''),
  )
}

async function fire(target: EventTarget, type: string, key?: string) {
  await act(async () => {
    const event = new window.Event(type, { bubbles: true })
    if (key) Object.defineProperty(event, 'key', { value: key })
    target.dispatchEvent(event)
  })
}

describe('ConnectedApps', () => {
  it('starts collapsed with three marks and +4', () => {
    expect(trigger().getAttribute('aria-expanded')).toBe('false')
    expect(listedApps()).toEqual([])
    expect(trigger().children).toHaveLength(4)
    expect(trigger().lastElementChild?.textContent).toBe('+4')
  })

  it('clicking opens a popover listing every app by name', async () => {
    await fire(trigger(), 'click')
    expect(trigger().getAttribute('aria-expanded')).toBe('true')
    expect(listedApps()).toEqual(SEVEN)
    const popover = document.getElementById(
      trigger().getAttribute('aria-controls') ?? '',
    )
    expect(popover?.textContent).toContain('Connected apps')
  })

  it('Escape closes it and keeps focus on the trigger', async () => {
    await fire(trigger(), 'click')
    await fire(document, 'keydown', 'Escape')
    expect(trigger().getAttribute('aria-expanded')).toBe('false')
    expect(listedApps()).toEqual([])
  })

  it('puts the restart hint in the popover, not beside Connected', async () => {
    expect(container.textContent).not.toContain('restart it once')
    await fire(trigger(), 'click')
    const popover = document.getElementById(
      trigger().getAttribute('aria-controls') ?? '',
    )
    expect(popover?.lastElementChild?.textContent).toBe(
      'If one is open, restart it once.',
    )
  })

  it('Escape closes a hover-opened popover and handing focus back does not reopen it', async () => {
    // focus() dispatches focusin synchronously in a browser; linkedom does not.
    const button = trigger()
    button.focus = () => {
      button.dispatchEvent(new window.Event('focusin', { bubbles: true }))
    }
    await fire(button, 'focusin')
    expect(trigger().getAttribute('aria-expanded')).toBe('true')
    await fire(document, 'keydown', 'Escape')
    expect(trigger().getAttribute('aria-expanded')).toBe('false')
    expect(listedApps()).toEqual([])
  })

  it('stacks the leftmost mark on top', () => {
    const marks = [...trigger().children] as HTMLElement[]
    const layers = marks.slice(0, 3).map((mark) => Number(mark.style.zIndex))
    expect(layers).toEqual([...layers].sort((a, b) => b - a))
  })

  it('a click outside closes it', async () => {
    await fire(trigger(), 'click')
    await fire(document.getElementById('outside') as HTMLElement, 'pointerdown')
    expect(listedApps()).toEqual([])
  })
})
