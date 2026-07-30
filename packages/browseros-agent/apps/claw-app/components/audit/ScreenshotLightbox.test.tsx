import { afterEach, beforeEach, describe, expect, it, mock } from 'bun:test'
import { parseHTML } from 'linkedom'
import { act, type ComponentProps, type ReactNode } from 'react'
import type { Root } from 'react-dom/client'
import * as _dialog from '@/components/ui/dialog'
import * as _auditHooks from '@/modules/api/audit.hooks'

mock.module('@/modules/api/audit.hooks', () => ({
  ..._auditHooks,
  useTaskScreenshotBaseUrl: () => 'http://127.0.0.1:9200',
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
  for (const [name, descriptor] of globalDescriptors) {
    if (descriptor) Object.defineProperty(globalThis, name, descriptor)
    else Reflect.deleteProperty(globalThis, name)
  }
  Reflect.deleteProperty(globalThis, 'IS_REACT_ACT_ENVIRONMENT')
})

describe('ScreenshotLightbox', () => {
  it('blocks the opened screenshot portal from PostHog replay', async () => {
    await act(async () => {
      root.render(
        <ScreenshotLightbox
          sessionId="session-private"
          screenshotId={42}
          sourceUrl="https://private.example/secret"
          offsetMs={1200}
          onClose={() => undefined}
        />,
      )
    })

    const dialog = container.querySelector('[data-slot="dialog-content"]')
    if (!dialog) {
      throw new Error(`dialog portal missing: ${document.body.outerHTML}`)
    }
    expect(dialog.getAttribute('class') ?? '').toContain('ph-no-capture')
    expect(dialog.textContent).toContain('private.example')
    expect(dialog.querySelector('img')?.getAttribute('src')).toContain(
      '/sessions/session-private/screenshots/42',
    )
  })
})
