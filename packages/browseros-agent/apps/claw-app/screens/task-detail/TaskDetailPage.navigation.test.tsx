import { afterEach, beforeEach, describe, expect, it, mock } from 'bun:test'
import { parseHTML } from 'linkedom'
import { act, type ComponentProps, type ReactNode } from 'react'
import type { Root } from 'react-dom/client'
import { MemoryRouter, Route, Routes } from 'react-router'
import * as _dialog from '@/components/ui/dialog'
import * as _auditHooks from '@/modules/api/audit.hooks'
import type { TaskDetailScreenData } from './task-detail.data'

mock.module('@/modules/api/audit.hooks', () => ({
  ..._auditHooks,
  useTaskScreenshotBaseUrl: () => 'http://127.0.0.1:9200',
}))

// embla measures the DOM, which linkedom has no layout for. Mock it at the
// boundary with an index-based fake so button/keyboard navigation drives the
// carousel the way a real swipe would.
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

mock.module('@/components/ui/dialog', () => ({
  ..._dialog,
  Dialog: ({ children }: { children?: ReactNode }) => <>{children}</>,
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

mock.module('@/components/audit/TaskHeader', () => ({
  TaskHeader: () => <div data-testid="task-header" />,
}))

mock.module('@/components/ui/tabs-auto-hide', () => ({
  AutoHideTabs: ({
    items,
  }: {
    items: Array<{ id: string; content: ReactNode }>
  }) => (
    <>
      {items.map((item) => (
        <section key={item.id} data-testid={`tab-${item.id}`}>
          {item.content}
        </section>
      ))}
    </>
  ),
}))

mock.module('./TabView', () => ({
  TabView: ({
    group,
    onScreenshotClick,
  }: {
    group: {
      id: string
      screenshots: Array<{ screenshotId: number }>
    }
    onScreenshotClick: (screenshotId: number) => void
  }) => (
    <div data-testid={`group-${group.id}`}>
      {group.screenshots.map((screenshot) => (
        <button
          key={screenshot.screenshotId}
          type="button"
          data-testid={`open-${group.id}-${screenshot.screenshotId}`}
          onClick={() => onScreenshotClick(screenshot.screenshotId)}
        >
          Open {screenshot.screenshotId}
        </button>
      ))}
    </div>
  ),
}))

const STARTED_AT = 1_000
const SCREENSHOTS = [10, 20, 30, 40, 50].map((screenshotId) => ({
  screenshotId,
  capturedAt: STARTED_AT + screenshotId * 10,
  toolName: 'screenshot',
}))

function screenData(moveLastScreenshot = false): TaskDetailScreenData {
  const pageByScreenshotId = new Map([
    [10, 1],
    [20, 2],
    [30, 1],
    [40, 2],
    [50, moveLastScreenshot ? 2 : 1],
  ])
  return {
    detail: {
      session: {
        sessionId: 'session-navigation',
        slug: 'codex',
        label: 'Codex',
        name: 'Navigation fixture',
        site: 'example.test',
        startedAt: STARTED_AT,
        endedAt: STARTED_AT + 1_000,
        durationMs: 1_000,
        dispatchCount: SCREENSHOTS.length,
        toolSequence: ['screenshot'],
        status: 'done',
        errorCount: 0,
        latestScreenshotId: 50,
      },
      dispatches: SCREENSHOTS.map((screenshot) => ({
        dispatchId: screenshot.screenshotId,
        createdAt: screenshot.capturedAt,
        slug: 'codex',
        label: 'Codex',
        sessionId: 'session-navigation',
        toolName: 'screenshot',
        pageId: pageByScreenshotId.get(screenshot.screenshotId),
        url: `https://${screenshot.screenshotId}.example/path`,
        durationMs: 5,
        screenshotId: screenshot.screenshotId,
      })),
    },
    screenshots: SCREENSHOTS,
    isPending: false,
    isError: false,
    error: null,
  }
}

let dataOverride = screenData()

mock.module('./task-detail.data', () => ({
  useTaskDetailScreenData: () => dataOverride,
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

const { TaskDetailPage } = await import('./TaskDetailPage')

let root: Root
let container: HTMLElement

beforeEach(async () => {
  embla = makeEmbla()
  dataOverride = screenData()
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
  for (const [name, descriptor] of globalDescriptors) {
    if (descriptor) Object.defineProperty(globalThis, name, descriptor)
    else Reflect.deleteProperty(globalThis, name)
  }
  Reflect.deleteProperty(globalThis, 'IS_REACT_ACT_ENVIRONMENT')
})

function Page() {
  return (
    <MemoryRouter initialEntries={['/audit/session-navigation']}>
      <Routes>
        <Route path="/audit/:sessionId" element={<TaskDetailPage />} />
      </Routes>
    </MemoryRouter>
  )
}

async function renderPage() {
  await act(async () => root.render(<Page />))
}

async function click(element: Element) {
  await act(async () => {
    element.dispatchEvent(new window.Event('click', { bubbles: true }))
  })
}

function getByTestId(testId: string): HTMLElement {
  const element = container.querySelector<HTMLElement>(
    `[data-testid="${testId}"]`,
  )
  if (!element) throw new Error(`missing test id: ${testId}`)
  return element
}

function getDialog(): HTMLElement {
  const dialog = container.querySelector<HTMLElement>(
    '[data-slot="dialog-content"]',
  )
  if (!dialog) throw new Error('missing screenshot lightbox')
  return dialog
}

function counter(): string {
  const span = Array.from(getDialog().querySelectorAll('span')).find((node) =>
    /^\d+ \/ \d+$/.test(node.textContent?.trim() ?? ''),
  )
  return span?.textContent?.trim() ?? ''
}

function getNavigationButton(label: string): HTMLButtonElement {
  const button = getDialog().querySelector<HTMLButtonElement>(
    `button[aria-label="${label}"]`,
  )
  if (!button) throw new Error(`missing navigation button: ${label}`)
  return button
}

describe('TaskDetailPage screenshot navigation', () => {
  it('opens on the clicked screenshot and lets the user swipe the whole session across groups', async () => {
    await renderPage()

    // Open a screenshot that lives in the "page-1" group.
    await click(getByTestId('open-page-1-30'))
    expect(getDialog().textContent).toContain('30.example · T+300ms')
    expect(counter()).toBe('3 / 5')
    expect(getDialog().querySelectorAll('img')).toHaveLength(5)

    // Next crosses into a screenshot owned by the other group (40 is page-2).
    await click(getNavigationButton('Next screenshot'))
    expect(getDialog().textContent).toContain('40.example · T+400ms')
    expect(counter()).toBe('4 / 5')

    await click(getNavigationButton('Next screenshot'))
    expect(counter()).toBe('5 / 5')
    expect(getNavigationButton('Next screenshot').disabled).toBe(true)
  })

  it('keeps the carousel position through a background poll re-render', async () => {
    await renderPage()

    await click(getByTestId('open-page-1-30'))
    await click(getNavigationButton('Next screenshot'))
    expect(counter()).toBe('4 / 5')

    // A poll moves the last screenshot to another tab group; the flat session
    // carousel is unaffected and the current position is preserved.
    dataOverride = screenData(true)
    await renderPage()

    expect(counter()).toBe('4 / 5')
    expect(getDialog().textContent).toContain('40.example · T+400ms')
    expect(getDialog().querySelectorAll('img')).toHaveLength(5)
  })
})
