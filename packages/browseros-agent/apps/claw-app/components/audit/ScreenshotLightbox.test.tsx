import { afterEach, beforeEach, describe, expect, it, mock } from 'bun:test'
import { parseHTML } from 'linkedom'
import { act, type ComponentProps, type ReactNode, useState } from 'react'
import type { Root } from 'react-dom/client'
import * as _dialog from '@/components/ui/dialog'
import * as _auditHooks from '@/modules/api/audit.hooks'

mock.module('@/modules/api/audit.hooks', () => ({
  ..._auditHooks,
  useTaskScreenshotBaseUrl: () => 'http://127.0.0.1:9200',
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

const ORDERED_IDS = [11, 22, 33] as const
const META = {
  11: { sourceUrl: 'https://first.example/start', offsetMs: 400 },
  22: { sourceUrl: 'https://second.example/middle', offsetMs: 4300 },
  33: { sourceUrl: 'https://third.example/end', offsetMs: 11700 },
} as const

interface ControlledLightboxProps {
  initialId?: number
  screenshotIds?: readonly number[]
  onNavigate?: (screenshotId: number) => void
  onClose?: () => void
}

function ControlledLightbox({
  initialId = 22,
  screenshotIds = ORDERED_IDS,
  onNavigate = () => undefined,
  onClose = () => undefined,
}: ControlledLightboxProps) {
  const [screenshotId, setScreenshotId] = useState(initialId)
  const meta = META[screenshotId as keyof typeof META] ?? {
    sourceUrl: 'https://unlisted.example/current',
    offsetMs: 9900,
  }
  return (
    <ScreenshotLightbox
      sessionId="session-navigation"
      screenshotId={screenshotId}
      screenshotIds={screenshotIds}
      sourceUrl={meta.sourceUrl}
      offsetMs={meta.offsetMs}
      onNavigate={(nextId) => {
        onNavigate(nextId)
        setScreenshotId(nextId)
      }}
      onClose={onClose}
    />
  )
}

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

function getButton(label: string): HTMLButtonElement {
  const button = container.querySelector<HTMLButtonElement>(
    `button[aria-label="${label}"]`,
  )
  if (!button) throw new Error(`button missing: ${label}`)
  return button
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
        screenshotId={42}
        screenshotIds={[42]}
        sourceUrl="https://private.example/secret"
        offsetMs={1200}
        onNavigate={() => undefined}
        onClose={() => undefined}
      />,
    )

    const dialog = getDialog()
    expect(dialog.getAttribute('class') ?? '').toContain('ph-no-capture')
    expect(dialog.getAttribute('class') ?? '').toContain('sm:max-w-[94vw]')
    expect(dialog.textContent).toContain('private.example')
    expect(dialog.textContent).toContain('1 / 1')
    const image = dialog.querySelector('img')
    expect(image?.getAttribute('src')).toContain(
      '/sessions/session-private/screenshots/42',
    )
    const imageClass = image?.getAttribute('class') ?? ''
    expect(imageClass).toContain('max-h-[calc(92vh-3.5rem)]')
    expect(imageClass).toContain('w-auto')
    expect(imageClass).toContain('max-w-[94vw]')
    expect(imageClass).toContain('object-contain')
    const previous = getButton('Previous screenshot')
    expect(previous.getAttribute('type')).toBe('button')
    expect(previous.getAttribute('class') ?? '').toContain('focus-visible')
    const position = Array.from(dialog.querySelectorAll('span')).find(
      (span) => span.textContent?.trim() === '1 / 1',
    )
    expect(position?.getAttribute('class') ?? '').toContain('tabular-nums')
    expect(position?.getAttribute('class') ?? '').toContain('min-w-[7ch]')
    const toolbar = previous.parentElement?.parentElement
    expect(toolbar?.nextElementSibling).toBe(image)
    expect(previous.disabled).toBe(true)
    expect(getButton('Next screenshot').disabled).toBe(true)
  })

  it('moves to adjacent ids and updates the image, caption, alt, and position together', async () => {
    const onNavigate = mock((_screenshotId: number) => undefined)
    await render(<ControlledLightbox onNavigate={onNavigate} />)

    expect(getDialog().textContent).toContain('second.example · T+4.3s')
    expect(getDialog().textContent).toContain('2 / 3')
    expect(getDialog().querySelector('img')?.getAttribute('alt')).toBe(
      'Screenshot of second.example',
    )

    await click(getButton('Next screenshot'))
    expect(onNavigate.mock.calls[0]?.[0]).toBe(33)
    expect(getDialog().textContent).toContain('third.example · T+11.7s')
    expect(getDialog().textContent).toContain('3 / 3')
    expect(getDialog().querySelector('img')?.getAttribute('src')).toContain(
      '/sessions/session-navigation/screenshots/33',
    )
    expect(getDialog().querySelector('img')?.getAttribute('alt')).toBe(
      'Screenshot of third.example',
    )

    await click(getButton('Previous screenshot'))
    await click(getButton('Previous screenshot'))
    expect(onNavigate.mock.calls.map((call) => call[0])).toEqual([33, 22, 11])
    expect(getDialog().textContent).toContain('first.example · T+400ms')
    expect(getDialog().textContent).toContain('1 / 3')
  })

  it('disables both boundaries without wrapping or firing navigation', async () => {
    const onNavigate = mock((_screenshotId: number) => undefined)
    await render(<ControlledLightbox initialId={11} onNavigate={onNavigate} />)

    const previous = getButton('Previous screenshot')
    expect(previous.disabled).toBe(true)
    await click(previous)
    expect(onNavigate).toHaveBeenCalledTimes(0)

    await click(getButton('Next screenshot'))
    await click(getButton('Next screenshot'))
    const next = getButton('Next screenshot')
    expect(next.disabled).toBe(true)
    expect(onNavigate).toHaveBeenCalledTimes(2)
    await click(next)
    expect(onNavigate).toHaveBeenCalledTimes(2)
    expect(getDialog().textContent).toContain('3 / 3')
  })

  it('navigates on bare arrow keys and leaves modified, composing, and Escape keys alone', async () => {
    const onNavigate = mock((_screenshotId: number) => undefined)
    await render(<ControlledLightbox onNavigate={onNavigate} />)

    const right = await pressKey(getDialog(), 'ArrowRight')
    expect(right.defaultPrevented).toBe(true)
    expect(onNavigate.mock.calls[0]?.[0]).toBe(33)

    const left = await pressKey(getDialog(), 'ArrowLeft')
    expect(left.defaultPrevented).toBe(true)
    expect(onNavigate.mock.calls[1]?.[0]).toBe(22)

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
    expect(onNavigate).toHaveBeenCalledTimes(2)
    for (const event of modifiedEvents) {
      expect(event.defaultPrevented).toBe(false)
    }
    expect(composing.defaultPrevented).toBe(false)
    expect(escapeEvent.defaultPrevented).toBe(false)
  })

  it('does not hijack arrow keys from editable or arrow-driven controls', async () => {
    const onNavigate = mock((_screenshotId: number) => undefined)
    await render(<ControlledLightbox onNavigate={onNavigate} />)
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
    expect(onNavigate).toHaveBeenCalledTimes(0)
  })

  it('keeps an active id missing from the polled list previewable as 1 / 1', async () => {
    const onNavigate = mock((_screenshotId: number) => undefined)
    await render(
      <ControlledLightbox
        initialId={99}
        screenshotIds={[11, 22]}
        onNavigate={onNavigate}
      />,
    )

    expect(getDialog().textContent).toContain('unlisted.example · T+9.9s')
    expect(getDialog().textContent).toContain('1 / 1')
    expect(getDialog().querySelector('img')?.getAttribute('src')).toContain(
      '/sessions/session-navigation/screenshots/99',
    )
    expect(getButton('Previous screenshot').disabled).toBe(true)
    expect(getButton('Next screenshot').disabled).toBe(true)
    expect(onNavigate).toHaveBeenCalledTimes(0)
  })

  it('preserves the close callback contract through dialog open changes', async () => {
    const onClose = mock(() => undefined)
    await render(<ControlledLightbox onClose={onClose} />)

    await act(async () => dialogOnOpenChange?.(true))
    expect(onClose).toHaveBeenCalledTimes(0)
    await act(async () => dialogOnOpenChange?.(false))
    expect(onClose).toHaveBeenCalledTimes(1)
  })
})
