import { afterEach, beforeEach, describe, expect, it, mock } from 'bun:test'
import { parseHTML } from 'linkedom'
import { act, type ComponentProps, type ReactNode } from 'react'
import type { Root } from 'react-dom/client'
import * as _dialog from '@/components/ui/dialog'

// Flatten the portal so the popup lands in the linkedom container.
mock.module('@/components/ui/dialog', () => ({
  ..._dialog,
  Dialog: ({ children, open }: { children?: ReactNode; open?: boolean }) =>
    open ? children : null,
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
  DialogDescription: (props: ComponentProps<'p'>) => <p {...props} />,
}))

const globalDescriptors = new Map(
  ['window', 'document', 'navigator', 'HTMLElement', 'Node', 'Event'].map(
    (name) => [name, Object.getOwnPropertyDescriptor(globalThis, name)],
  ),
)

const { InstallExtensionDialog } = await import('./InstallExtensionDialog')
const {
  COWORK_REQUIREMENT_LINE,
  EXTENSION_DOWNLOAD_URL,
  EXTENSION_RELEASES_URL,
  INSTALL_STEPS,
} = await import('./install-guide.data')

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

function buttonWithText(text: string): HTMLElement {
  const match = [...container.querySelectorAll('button')].find(
    (button) => (button.textContent ?? '').trim() === text,
  )
  if (!match) {
    throw new Error(
      `no button "${text}" among: ${[...container.querySelectorAll('button')]
        .map((button) => `"${(button.textContent ?? '').trim()}"`)
        .join(', ')}`,
    )
  }
  return match as unknown as HTMLElement
}

function railButton(index: number): HTMLElement {
  const match = container.querySelectorAll('nav ol li button')[index]
  if (!match) throw new Error(`no rail entry at index ${index}`)
  return match as unknown as HTMLElement
}

async function click(element: HTMLElement) {
  await act(async () => {
    element.dispatchEvent(new window.Event('click', { bubbles: true }))
  })
}

function paneText(): string {
  return container.textContent ?? ''
}

async function open() {
  await act(async () => {
    root.render(<InstallExtensionDialog open onOpenChange={() => undefined} />)
  })
}

describe('InstallExtensionDialog', () => {
  it('opens on step 1 with its screenshot, the Cowork line, and the counter', async () => {
    await open()

    const first = INSTALL_STEPS[0]
    expect(paneText()).toContain(first.title)
    expect(paneText()).toContain(first.body)
    expect(paneText()).toContain(COWORK_REQUIREMENT_LINE)
    expect(paneText()).toContain(`step 1 of ${INSTALL_STEPS.length}`)
    expect(container.querySelector('img')?.getAttribute('src')).toBe(
      first.image?.src,
    )
  })

  it('lists every step in the rail and jumps straight to the one clicked', async () => {
    await open()
    for (const step of INSTALL_STEPS) {
      expect(paneText()).toContain(step.title)
    }

    const target = INSTALL_STEPS[3]
    await click(railButton(3))

    expect(paneText()).toContain(`step 4 of ${INSTALL_STEPS.length}`)
    expect(paneText()).toContain(target.body)
    expect(container.querySelector('img')?.getAttribute('src')).toBe(
      target.image?.src,
    )
  })

  it('advances with Next, returns with Back, and pins Back on the first step', async () => {
    await open()
    expect(buttonWithText('Back').hasAttribute('disabled')).toBe(true)

    await click(buttonWithText('Next'))
    expect(paneText()).toContain(`step 2 of ${INSTALL_STEPS.length}`)
    expect(buttonWithText('Back').hasAttribute('disabled')).toBe(false)

    await click(buttonWithText('Back'))
    expect(paneText()).toContain(`step 1 of ${INSTALL_STEPS.length}`)
  })

  it('swaps the screenshot for the direct .mcpb download on the download step', async () => {
    await open()
    const downloadIndex = INSTALL_STEPS.findIndex(
      (step) => step.kind === 'download',
    )
    expect(downloadIndex).toBeGreaterThan(-1)
    await click(railButton(downloadIndex))

    expect(container.querySelector('img')).toBeNull()
    const hrefs = [...container.querySelectorAll('a')].map((anchor) =>
      anchor.getAttribute('href'),
    )
    expect(hrefs).toContain(EXTENSION_DOWNLOAD_URL)
    expect(hrefs).toContain(EXTENSION_RELEASES_URL)
  })

  it('ends on a Done control that closes the dialog', async () => {
    const openChanges: boolean[] = []
    await act(async () => {
      root.render(
        <InstallExtensionDialog
          open
          onOpenChange={(next) => openChanges.push(next)}
        />,
      )
    })

    await click(railButton(INSTALL_STEPS.length - 1))
    expect(() => buttonWithText('Next')).toThrow()

    await click(buttonWithText('Done'))
    expect(openChanges).toEqual([false])
  })

  it('restarts the walkthrough when reopened', async () => {
    await open()
    await click(buttonWithText('Next'))
    expect(paneText()).toContain(`step 2 of ${INSTALL_STEPS.length}`)

    await act(async () => {
      root.render(
        <InstallExtensionDialog open={false} onOpenChange={() => undefined} />,
      )
    })
    await open()

    expect(paneText()).toContain(`step 1 of ${INSTALL_STEPS.length}`)
  })

  it('overrides the dialog width clamp at the sm breakpoint', async () => {
    await open()
    const surface = container.querySelector('[data-slot="dialog-content"]')
    const classes = surface?.getAttribute('class') ?? ''
    // A base-only max-w is silently ignored at >=640px, so both must be set.
    expect(classes).toContain('max-w-[94vw]')
    expect(classes).toContain('sm:max-w-[min(58rem,94vw)]')
  })
})
