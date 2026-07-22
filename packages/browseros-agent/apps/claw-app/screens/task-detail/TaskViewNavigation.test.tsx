import { afterEach, beforeEach, describe, expect, it, mock } from 'bun:test'
import { parseHTML } from 'linkedom'
import {
  act,
  createContext,
  type HTMLAttributes,
  type ReactNode,
  useContext,
} from 'react'
import type { Root } from 'react-dom/client'

interface SelectContextValue {
  onValueChange: (value: string) => void
}

const SelectContext = createContext<SelectContextValue | null>(null)

mock.module('@/components/ui/select', () => ({
  Select: ({
    children,
    onValueChange,
  }: {
    children: ReactNode
    value: string
    onValueChange: (value: string) => void
  }) => (
    <SelectContext.Provider value={{ onValueChange }}>
      {children}
    </SelectContext.Provider>
  ),
  SelectTrigger: ({
    children,
    ...props
  }: React.ButtonHTMLAttributes<HTMLButtonElement>) => (
    <button
      data-slot="select-trigger"
      type="button"
      role="combobox"
      aria-expanded="false"
      {...props}
    >
      {children}
    </button>
  ),
  SelectValue: ({ children, ...props }: HTMLAttributes<HTMLSpanElement>) => (
    <span data-slot="select-value" {...props}>
      {children}
    </span>
  ),
  SelectContent: ({
    align: _align,
    alignItemWithTrigger: _alignItemWithTrigger,
    children,
    ...props
  }: HTMLAttributes<HTMLDivElement> & {
    align?: string
    alignItemWithTrigger?: boolean
  }) => (
    <div data-slot="select-content" {...props}>
      {children}
    </div>
  ),
  SelectItem: ({
    children,
    label,
    value,
  }: {
    children: ReactNode
    label?: string
    value: string
  }) => {
    const context = useContext(SelectContext)
    return (
      <button
        data-slot="select-item"
        type="button"
        role="option"
        aria-label={label}
        onClick={() => context?.onValueChange(value)}
      >
        {children}
      </button>
    )
  },
}))

const { TaskViewNavigation } = await import('./TaskViewNavigation')
type TaskViewNavigationItem =
  import('./TaskViewNavigation').TaskViewNavigationItem

const globalNames = [
  'window',
  'document',
  'navigator',
  'Element',
  'HTMLElement',
  'Node',
  'Event',
  'MouseEvent',
  'KeyboardEvent',
  'getComputedStyle',
  'ResizeObserver',
] as const

const globalDescriptors = new Map(
  globalNames.map((name) => [
    name,
    Object.getOwnPropertyDescriptor(globalThis, name),
  ]),
)

let root: Root
let container: HTMLElement

function item(
  id: string,
  label: string,
  count: number,
): TaskViewNavigationItem {
  return {
    id,
    label,
    count,
    content: <div data-view-content={id}>{label} content</div>,
  }
}

async function render(
  items: TaskViewNavigationItem[],
  defaultId?: string,
): Promise<void> {
  await act(async () => {
    root.render(<TaskViewNavigation items={items} defaultId={defaultId} />)
  })
}

function dispatchMouse(target: Element, type: string): void {
  const event = new window.Event(type, { bubbles: true, cancelable: true })
  Object.defineProperties(event, {
    button: { value: 0 },
    buttons: { value: type === 'mouseup' ? 0 : 1 },
    clientX: { value: 0 },
    clientY: { value: 0 },
    detail: { value: 1 },
    pointerType: { value: 'mouse' },
  })
  target.dispatchEvent(event)
}

beforeEach(async () => {
  const dom = parseHTML(
    '<!doctype html><html><body><div id="root"></div></body></html>',
  )
  const ResizeObserver = class {
    observe(): void {}
    unobserve(): void {}
    disconnect(): void {}
  }
  const globals = {
    window: dom.window,
    document: dom.document,
    navigator: dom.window.navigator,
    Element: dom.window.Element,
    HTMLElement: dom.window.HTMLElement,
    Node: dom.window.Node,
    Event: dom.window.Event,
    MouseEvent: dom.window.MouseEvent ?? dom.window.Event,
    KeyboardEvent: dom.window.KeyboardEvent ?? dom.window.Event,
    getComputedStyle: () => ({
      direction: 'ltr',
      getPropertyValue: () => '',
    }),
    ResizeObserver,
  }
  for (const [name, value] of Object.entries(globals)) {
    Object.defineProperty(globalThis, name, {
      configurable: true,
      writable: true,
      value,
    })
  }
  Object.assign(dom.window, {
    requestAnimationFrame: (callback: FrameRequestCallback) => {
      callback(0)
      return 1
    },
    cancelAnimationFrame: () => undefined,
  })
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

describe('TaskViewNavigation', () => {
  it('renders nothing for zero items and direct content for one item', async () => {
    await render([])
    expect(container.innerHTML).toBe('')

    await render([item('session', 'Session', 4)])
    expect(container.textContent).toContain('Session content')
    expect(container.querySelector('[data-slot="select-trigger"]')).toBeNull()
  })

  it('shows the selected label and count in one width-bounded control', async () => {
    await render(
      [item('session', 'Session', 111), item('page-7', 'Tab 1', 6)],
      'session',
    )

    const navigation = container.querySelector('[data-task-view-navigation]')
    const trigger = container.querySelector('[data-slot="select-trigger"]')
    expect(navigation).not.toBeNull()
    expect(trigger?.textContent).toContain('Session')
    expect(trigger?.textContent).toContain('111')
    expect(trigger?.getAttribute('aria-label')).toBe('Audit view')
    expect(trigger?.getAttribute('class')).toContain('w-full')
    expect(trigger?.getAttribute('class')).toContain('max-w-72')
    expect(container.querySelectorAll('[role="combobox"]')).toHaveLength(1)
    expect(container.querySelector('[role="tablist"]')).toBeNull()
    expect(container.textContent).toContain('Session content')
    expect(container.textContent).not.toContain('Tab 1 content')
  })

  it('falls back to the first item when the requested default is absent', async () => {
    await render(
      [item('session', 'Session', 5), item('page-7', 'Tab 1', 2)],
      'missing',
    )

    expect(
      container.querySelector('[data-slot="select-trigger"]')?.textContent,
    ).toContain('Session')
    expect(container.textContent).toContain('Session content')
  })

  it('exposes every option in a scrollable popup and switches selected content', async () => {
    const items = [
      item('session', 'Session', 40),
      ...Array.from({ length: 20 }, (_, index) =>
        item(`page-${index + 1}`, `Tab ${index + 1}`, index + 1),
      ),
    ]
    await render(items, 'session')

    const popup = document.querySelector('[data-slot="select-content"]')
    expect(popup).not.toBeNull()
    expect(popup?.getAttribute('class')).toContain(
      'max-h-[min(18rem,var(--available-height))]',
    )
    expect(popup?.getAttribute('class')).toContain('overflow-y-auto')
    expect(document.querySelectorAll('[data-slot="select-item"]')).toHaveLength(
      21,
    )
    expect(document.body.textContent).toContain('Tab 20')
    expect(document.body.textContent).toContain('20')

    const tab20 = [
      ...document.querySelectorAll('[data-slot="select-item"]'),
    ].find((option) => option.textContent?.includes('Tab 20'))
    if (!tab20) throw new Error('Tab 20 option missing')
    await act(async () => {
      dispatchMouse(tab20, 'click')
    })

    expect(container.textContent).toContain('Tab 20 content')
    expect(container.textContent).not.toContain('Session content')
    expect(
      container.querySelector('[data-slot="select-trigger"]')?.textContent,
    ).toContain('Tab 20')
  })
})
