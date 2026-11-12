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
import {
  act,
  cloneElement,
  createContext,
  type ReactElement,
  type ReactNode,
  useContext,
} from 'react'
import type { Root } from 'react-dom/client'

const ModalOpen = createContext<{
  open: boolean
  setOpen: (open: boolean) => void
}>({ open: false, setOpen: () => undefined })

// The beUI modal animates through motion and measures the DOM, which linkedom
// cannot do. A plain dialog keeps the open state, the name and the close wiring.
mock.module('@/components/motion/center-morph-modal', () => ({
  CenterMorphModal: ({
    open,
    onOpenChange,
    children,
  }: {
    open?: boolean
    onOpenChange?: (open: boolean) => void
    children: ReactNode
  }) => (
    <ModalOpen.Provider
      value={{ open: Boolean(open), setOpen: (next) => onOpenChange?.(next) }}
    >
      {children}
    </ModalOpen.Provider>
  ),
  CenterMorphModalContent: ({
    ariaLabel,
    children,
  }: {
    ariaLabel: string
    children: ReactNode
  }) =>
    useContext(ModalOpen).open ? (
      <div role="dialog" aria-label={ariaLabel}>
        {children}
      </div>
    ) : null,
  CenterMorphModalClose: ({
    children,
  }: {
    children: ReactElement<{ onClick?: () => void }>
  }) => {
    const modal = useContext(ModalOpen)
    return cloneElement(children, {
      onClick: () => {
        children.props.onClick?.()
        modal.setOpen(false)
      },
    })
  },
}))

const { UltrafastTour } = await import('./UltrafastTour')

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
let closed = 0

function define(name: string, value: unknown) {
  Object.defineProperty(globalThis, name, {
    configurable: true,
    writable: true,
    value,
  })
}

beforeEach(async () => {
  closed = 0
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

async function show(props: { startAt?: number; welcome?: boolean }) {
  await act(async () => {
    root.render(
      <UltrafastTour
        open
        {...props}
        onClose={() => {
          closed += 1
        }}
      />,
    )
  })
}

function dialog(): Element {
  const found = container.querySelector('[role="dialog"]')
  if (!found) throw new Error('no dialog open')
  return found
}

function nameOf(element: Element): string {
  return (
    element.getAttribute('aria-label') ?? (element.textContent ?? '').trim()
  )
}

function button(label: string): HTMLElement {
  const buttons = [...container.querySelectorAll('button')]
  const match = buttons.find((candidate) => nameOf(candidate) === label)
  if (!match) {
    throw new Error(
      `no "${label}" button among: ${buttons.map((b) => `"${nameOf(b)}"`).join(', ')}`,
    )
  }
  return match as unknown as HTMLElement
}

async function click(label: string) {
  const target = button(label)
  await act(async () => {
    target.dispatchEvent(new window.Event('click', { bubbles: true }))
  })
}

async function press(key: string) {
  await act(async () => {
    const event = new window.Event('keydown', { bubbles: true })
    Object.defineProperty(event, 'key', { value: key })
    dialog().querySelector('h2')?.dispatchEvent(event)
  })
}

const headline = () => dialog().querySelector('h2')?.textContent

describe('UltrafastTour', () => {
  it('tests that the welcome opens on the first picture and says welcome', async () => {
    await show({ welcome: true })

    expect(dialog().getAttribute('aria-label')).toBe('Welcome to ultrafast')
    expect(headline()).toBe('Fewer steps. Smaller AI bills.')
    expect(dialog().textContent).toContain(
      'On carts and forms, neo does the clicking.',
    )
    expect(dialog().textContent).toContain(
      'Illustration. Each dot is a step your agent pays for.',
    )
    expect(button('Step 1 of 3').getAttribute('aria-current')).toBe('step')
    expect(() => button('Back')).toThrow()
    expect(button('Skip')).toBeTruthy()
  })

  it('tests that Next, Back and the dots move between the three pictures', async () => {
    await show({ welcome: true })

    await click('Next')
    expect(headline()).toBe('Skills run in seconds')
    expect(dialog().getAttribute('aria-label')).toBe('ultrafast skills')
    expect(dialog().textContent).toContain(
      'Illustration. You pick with or without your agent.',
    )

    await click('Step 3 of 3')
    expect(headline()).toBe('Robot checks, solved for you')
    expect(dialog().textContent).toContain('Solved')
    expect(() => button('Next')).toThrow()

    await click('Back')
    expect(headline()).toBe('Skills run in seconds')
  })

  it('tests that the arrow keys move between pictures and stop at the ends', async () => {
    await show({})

    await press('ArrowLeft')
    expect(headline()).toBe('Fewer steps. Smaller AI bills.')
    await press('ArrowRight')
    await press('ArrowRight')
    await press('ArrowRight')
    expect(headline()).toBe('Robot checks, solved for you')
  })

  it('tests that Skip closes from any picture', async () => {
    await show({ startAt: 1 })
    expect(headline()).toBe('Skills run in seconds')

    await click('Skip')
    expect(closed).toBe(1)
  })

  it('tests that the last picture finishes with Start using ultrafast', async () => {
    await show({ startAt: 2 })

    await click('Start using ultrafast')
    expect(closed).toBe(1)
  })

  it('tests that no picture shows a number', async () => {
    await show({})
    for (const step of ['Step 1 of 3', 'Step 2 of 3', 'Step 3 of 3']) {
      await click(step)
      const picture = dialog().querySelector('figure')
      expect(picture?.textContent).toContain('Illustration.')
      expect(picture?.textContent).not.toMatch(/\d/)
    }
  })
})
