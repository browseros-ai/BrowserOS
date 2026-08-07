import { afterEach, beforeEach, describe, expect, it, mock } from 'bun:test'
import { parseHTML } from 'linkedom'
import { act, type ComponentProps, type ReactNode } from 'react'
import type { Root } from 'react-dom/client'
import * as _dialog from '@/components/ui/dialog'
import * as _auditHooks from '@/modules/api/audit.hooks'
import type { ScreenshotLightboxItem } from './ScreenshotLightbox'

mock.module('@/modules/api/audit.hooks', () => ({
  ..._auditHooks,
  useTaskScreenshotBaseUrl: () => 'http://127.0.0.1:9200',
}))

// embla measures the DOM, which linkedom has no layout for. Mock it at the
// boundary with an index-based fake so the toolbar/counter wiring is exercised
// the way a real swipe would drive it (select events, clamped scrolls).
type Listener = () => void
interface EmblaFake {
  index: number
  count: number
  initialized: boolean
  listeners: Map<string, Listener[]>
  api: Record<string, unknown>
  ref: (node: Element | null) => void
}

function makeEmbla(): EmblaFake {
  const listeners = new Map<string, Listener[]>()
  const emit = (name: string) => {
    for (const cb of listeners.get(name) ?? []) cb()
  }
  const state: EmblaFake = {
    index: 0,
    count: 0,
    initialized: false,
    listeners,
    api: {},
    ref: () => undefined,
  }
  state.api = {
    scrollNext: () => {
      if (state.index < state.count - 1) {
        state.index += 1
        emit('select')
      }
    },
    scrollPrev: () => {
      if (state.index > 0) {
        state.index -= 1
        emit('select')
      }
    },
    scrollTo: (i: number) => {
      state.index = Math.max(0, Math.min(i, Math.max(0, state.count - 1)))
      emit('select')
    },
    canScrollNext: () => state.index < state.count - 1,
    canScrollPrev: () => state.index > 0,
    selectedScrollSnap: () => state.index,
    on: (name: string, cb: Listener) => {
      const arr = listeners.get(name) ?? []
      arr.push(cb)
      listeners.set(name, arr)
    },
    off: (name: string, cb: Listener) => {
      listeners.set(
        name,
        (listeners.get(name) ?? []).filter((entry) => entry !== cb),
      )
    },
  }
  state.ref = (node) => {
    if (node) {
      state.count = node.querySelectorAll('[data-slot="carousel-item"]').length
    }
  }
  return state
}

let embla = makeEmbla()

mock.module('embla-carousel-react', () => ({
  default: (options?: { startIndex?: number }) => {
    if (!embla.initialized) {
      embla.initialized = true
      if (options?.startIndex != null) embla.index = options.startIndex
    }
    return [embla.ref, embla.api]
  },
}))

let dialogOnOpenChange: ((open: boolean) => void) | undefined

mock.module('@/components/ui/dialog', () => ({
  ..._dialog,
  Dialog: ({
    children,
    onOpenChange,
  }: {
    children?: ReactNode
    onOpenChange?: (open: boolean) => void
  }) => {
    dialogOnOpenChange = onOpenChange
    return <>{children}</>
  },
  DialogClose: ({ children }: { children?: ReactNode }) => <>{children}</>,
  DialogContent: ({
    children,
    showCloseButton: _showCloseButton,
    ...props
  }: ComponentProps<'div'> & { showCloseButton?: boolean }) => (
    <div data-slot="dialog-content" {...props}>
      {children}
    </div>
  ),
  DialogTitle: (props: ComponentProps<'h2'>) => <h2 {...props} />,
}))

const GLOBAL_NAMES = [
  'window',
  'document',
  'navigator',
  'HTMLElement',
  'Element',
  'Node',
  'Event',
] as const
const globalDescriptors = new Map(
  GLOBAL_NAMES.map((name) => [
    name,
    Object.getOwnPropertyDescriptor(globalThis, name),
  ]),
)

const { ScreenshotLightbox } = await import('./ScreenshotLightbox')

let root: Root
let container: HTMLElement

beforeEach(async () => {
  embla = makeEmbla()
  const dom = parseHTML(
    '<!doctype html><html><body><div id="root"></div></body></html>',
  )
  const globals = {
    window: dom.window,
    document: dom.document,
    navigator: dom.window.navigator,
    HTMLElement: dom.window.HTMLElement,
    Element: dom.window.Element,
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
  dialogOnOpenChange = undefined
  for (const [name, descriptor] of globalDescriptors) {
    if (descriptor) Object.defineProperty(globalThis, name, descriptor)
    else Reflect.deleteProperty(globalThis, name)
  }
  Reflect.deleteProperty(globalThis, 'IS_REACT_ACT_ENVIRONMENT')
})

const ITEMS: ScreenshotLightboxItem[] = [
  { screenshotId: 11, sourceUrl: 'https://first.example/start', offsetMs: 400 },
  {
    screenshotId: 22,
    sourceUrl: 'https://second.example/middle',
    offsetMs: 4300,
  },
  { screenshotId: 33, sourceUrl: 'https://third.example/end', offsetMs: 11700 },
]

async function render(node: ReactNode) {
  await act(async () => root.render(node))
}

function getDialog(): HTMLElement {
  const dialog = container.querySelector<HTMLElement>(
    '[data-slot="dialog-content"]',
  )
  if (!dialog)
    throw new Error(`dialog portal missing: ${document.body.outerHTML}`)
  return dialog
}

function counter(): string {
  const span = Array.from(getDialog().querySelectorAll('span')).find((node) =>
    /^\d+ \/ \d+$/.test(node.textContent?.trim() ?? ''),
  )
  return span?.textContent?.trim() ?? ''
}

function getButton(label: string): HTMLButtonElement {
  const button = container.querySelector<HTMLButtonElement>(
    `button[aria-label="${label}"]`,
  )
  if (!button) throw new Error(`button missing: ${label}`)
  return button
}

function imgSrcs(): string[] {
  return Array.from(getDialog().querySelectorAll('img')).map(
    (img) => img.getAttribute('src') ?? '',
  )
}

async function click(element: Element) {
  await act(async () => {
    element.dispatchEvent(new window.Event('click', { bubbles: true }))
  })
}

interface KeyOptions {
  altKey?: boolean
  ctrlKey?: boolean
  metaKey?: boolean
  shiftKey?: boolean
  isComposing?: boolean
}

async function pressKey(
  target: Element,
  key: string,
  options: KeyOptions = {},
) {
  const event = new window.Event('keydown', {
    bubbles: true,
    cancelable: true,
  })
  Object.defineProperties(event, {
    key: { value: key },
    altKey: { value: options.altKey ?? false },
    ctrlKey: { value: options.ctrlKey ?? false },
    metaKey: { value: options.metaKey ?? false },
    shiftKey: { value: options.shiftKey ?? false },
    isComposing: { value: options.isComposing ?? false },
  })
  await act(async () => target.dispatchEvent(event))
  return event
}

describe('ScreenshotLightbox', () => {
  it('keeps the privacy and responsive-width controls on the opened portal', async () => {
    await render(
      <ScreenshotLightbox
        sessionId="session-private"
        items={[
          {
            screenshotId: 42,
            sourceUrl: 'https://private.example/secret',
            offsetMs: 1200,
          },
        ]}
        startId={42}
        onClose={() => undefined}
      />,
    )

    const dialog = getDialog()
    expect(dialog.getAttribute('class') ?? '').toContain('ph-no-capture')
    expect(dialog.getAttribute('class') ?? '').toContain('sm:max-w-[94vw]')
    expect(dialog.textContent).toContain('private.example')
    expect(counter()).toBe('1 / 1')

    const image = dialog.querySelector('img')
    expect(image?.getAttribute('src')).toContain(
      '/sessions/session-private/screenshots/42',
    )
    const imageClass = image?.getAttribute('class') ?? ''
    expect(imageClass).toContain('max-h-[calc(92vh-3.5rem)]')
    expect(imageClass).toContain('object-contain')

    const previous = getButton('Previous screenshot')
    expect(previous.getAttribute('type')).toBe('button')
    expect(previous.getAttribute('class') ?? '').toContain('focus-visible')
    const position = Array.from(dialog.querySelectorAll('span')).find(
      (span) => span.textContent?.trim() === '1 / 1',
    )
    expect(position?.getAttribute('class') ?? '').toContain('tabular-nums')
    expect(position?.getAttribute('class') ?? '').toContain('min-w-[7ch]')

    // The carousel region follows the toolbar (never overlays the image).
    const toolbar = previous.parentElement?.parentElement
    expect(toolbar?.nextElementSibling?.getAttribute('data-slot')).toBe(
      'carousel',
    )
    expect(previous.disabled).toBe(true)
    expect(getButton('Next screenshot').disabled).toBe(true)
  })

  it('renders every screenshot as a slide and opens on the clicked one', async () => {
    await render(
      <ScreenshotLightbox
        sessionId="session-navigation"
        items={ITEMS}
        startId={22}
        onClose={() => undefined}
      />,
    )

    const srcs = imgSrcs()
    expect(srcs).toHaveLength(3)
    expect(srcs.some((src) => src.includes('screenshots/11'))).toBe(true)
    expect(srcs.some((src) => src.includes('screenshots/22'))).toBe(true)
    expect(srcs.some((src) => src.includes('screenshots/33'))).toBe(true)

    expect(getDialog().textContent).toContain('second.example · T+4.3s')
    expect(counter()).toBe('2 / 3')
  })

  it('moves forward and back through the carousel, keeping caption and position in sync', async () => {
    await render(
      <ScreenshotLightbox
        sessionId="session-navigation"
        items={ITEMS}
        startId={22}
        onClose={() => undefined}
      />,
    )

    await click(getButton('Next screenshot'))
    expect(counter()).toBe('3 / 3')
    expect(getDialog().textContent).toContain('third.example · T+11.7s')

    await click(getButton('Previous screenshot'))
    await click(getButton('Previous screenshot'))
    expect(counter()).toBe('1 / 3')
    expect(getDialog().textContent).toContain('first.example · T+400ms')
  })

  it('disables both boundaries without wrapping', async () => {
    await render(
      <ScreenshotLightbox
        sessionId="session-navigation"
        items={ITEMS}
        startId={11}
        onClose={() => undefined}
      />,
    )

    const previous = getButton('Previous screenshot')
    expect(previous.disabled).toBe(true)
    await click(previous)
    expect(counter()).toBe('1 / 3')

    await click(getButton('Next screenshot'))
    await click(getButton('Next screenshot'))
    const next = getButton('Next screenshot')
    expect(next.disabled).toBe(true)
    await click(next)
    expect(counter()).toBe('3 / 3')
  })

  it('navigates on bare arrow keys and leaves modified, composing, and Escape keys alone', async () => {
    await render(
      <ScreenshotLightbox
        sessionId="session-navigation"
        items={ITEMS}
        startId={22}
        onClose={() => undefined}
      />,
    )

    const right = await pressKey(getDialog(), 'ArrowRight')
    expect(right.defaultPrevented).toBe(true)
    expect(counter()).toBe('3 / 3')

    const left = await pressKey(getDialog(), 'ArrowLeft')
    expect(left.defaultPrevented).toBe(true)
    expect(counter()).toBe('2 / 3')

    const modifiedEvents: Event[] = []
    for (const options of [
      { altKey: true },
      { ctrlKey: true },
      { metaKey: true },
      { shiftKey: true },
    ]) {
      modifiedEvents.push(await pressKey(getDialog(), 'ArrowRight', options))
    }
    const composing = await pressKey(getDialog(), 'ArrowRight', {
      isComposing: true,
    })
    const escapeEvent = await pressKey(getDialog(), 'Escape')
    for (const event of modifiedEvents) {
      expect(event.defaultPrevented).toBe(false)
    }
    expect(composing.defaultPrevented).toBe(false)
    expect(escapeEvent.defaultPrevented).toBe(false)
    expect(counter()).toBe('2 / 3')
  })

  it('does not hijack arrow keys from editable or arrow-driven controls', async () => {
    await render(
      <ScreenshotLightbox
        sessionId="session-navigation"
        items={ITEMS}
        startId={22}
        onClose={() => undefined}
      />,
    )
    const dialog = getDialog()

    const input = document.createElement('input')
    const textarea = document.createElement('textarea')
    const select = document.createElement('select')
    const editable = document.createElement('div')
    editable.setAttribute('contenteditable', 'true')
    const editableChild = document.createElement('span')
    editable.append(editableChild)
    const roleTargets = ['textbox', 'combobox', 'slider', 'spinbutton'].map(
      (role) => {
        const target = document.createElement('div')
        target.setAttribute('role', role)
        return target
      },
    )
    const protectedTargets = [
      input,
      textarea,
      select,
      editableChild,
      ...roleTargets,
    ]
    dialog.append(input, textarea, select, editable, ...roleTargets)

    for (const target of protectedTargets) {
      const event = await pressKey(target, 'ArrowRight')
      expect(event.defaultPrevented).toBe(false)
    }
    expect(counter()).toBe('2 / 3')
  })

  it('keeps the clicked screenshot visible solo when it is absent from the list', async () => {
    await render(
      <ScreenshotLightbox
        sessionId="session-navigation"
        items={ITEMS.slice(0, 2)}
        startId={99}
        onClose={() => undefined}
      />,
    )

    // The selection dropped out of the polled list (e.g. pruned mid-view): show
    // it alone rather than jumping to an unrelated screenshot.
    expect(counter()).toBe('1 / 1')
    expect(imgSrcs()).toEqual([
      expect.stringContaining('/sessions/session-navigation/screenshots/99'),
    ])
    expect(getButton('Previous screenshot').disabled).toBe(true)
    expect(getButton('Next screenshot').disabled).toBe(true)
  })

  it('eager-loads the active slide and its neighbors, lazy-loads the rest', async () => {
    const many: ScreenshotLightboxItem[] = [1, 2, 3, 4, 5].map((id) => ({
      screenshotId: id,
      sourceUrl: `https://site-${id}.example/page`,
      offsetMs: id * 1000,
    }))
    await render(
      <ScreenshotLightbox
        sessionId="session-lazy"
        items={many}
        startId={3}
        onClose={() => undefined}
      />,
    )

    const loading = Array.from(getDialog().querySelectorAll('img')).map((img) =>
      img.getAttribute('loading'),
    )
    expect(loading).toEqual(['lazy', 'eager', 'eager', 'eager', 'lazy'])
  })

  it('preserves the close callback contract through dialog open changes', async () => {
    const onClose = mock(() => undefined)
    await render(
      <ScreenshotLightbox
        sessionId="session-navigation"
        items={ITEMS}
        startId={22}
        onClose={onClose}
      />,
    )

    await act(async () => dialogOnOpenChange?.(true))
    expect(onClose).toHaveBeenCalledTimes(0)
    await act(async () => dialogOnOpenChange?.(false))
    expect(onClose).toHaveBeenCalledTimes(1)
  })
})
