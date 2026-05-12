import { afterEach, beforeEach, describe, expect, it, mock, vi } from 'bun:test'
import { createElement } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { Window } from 'happy-dom'

vi.mock('@/lib/utils', () => ({
  cn: (...args: unknown[]) =>
    args.filter((a) => typeof a === 'string' && a.length > 0).join(' '),
}))

vi.mock('@/components/ui/button', () => ({
  Button: ({ children, className = '', ...props }: Record<string, unknown>) =>
    createElement('button', { className, ...props }, children),
}))

vi.mock('@/components/ui/button-group', () => ({
  ButtonGroup: ({ children, className = '', ...props }: Record<string, unknown>) =>
    createElement('div', { className, ...props }, children),
  ButtonGroupText: ({ children, className = '', ...props }: Record<string, unknown>) =>
    createElement('span', { className, ...props }, children),
}))

vi.mock('@/components/ui/tooltip', () => ({
  TooltipProvider: ({ children }: Record<string, unknown>) => children,
  Tooltip: ({ children }: Record<string, unknown>) => children,
  TooltipTrigger: ({ children }: Record<string, unknown>) => children,
  TooltipContent: ({ children }: Record<string, unknown>) =>
    createElement('div', {}, children),
}))

vi.mock('lucide-react', () => ({
  ChevronLeftIcon: (props: Record<string, unknown>) => createElement('svg', props),
  ChevronRightIcon: (props: Record<string, unknown>) => createElement('svg', props),
  PaperclipIcon: (props: Record<string, unknown>) => createElement('svg', props),
  XIcon: (props: Record<string, unknown>) => createElement('svg', props),
}))

vi.mock('streamdown', () => ({
  Streamdown: ({ children, className = '', ...props }: Record<string, unknown>) =>
    createElement('div', { className, ...props }, children),
}))

const happyWindow = new Window()
globalThis.document = happyWindow.document as unknown as Document
globalThis.window = happyWindow as unknown as Window & typeof globalThis
globalThis.navigator = happyWindow.navigator as unknown as Navigator
const happyGlobal = happyWindow as unknown as Record<string, unknown>
if (!happyGlobal.SyntaxError) happyGlobal.SyntaxError = SyntaxError
if (!happyGlobal.TypeError) happyGlobal.TypeError = TypeError
if (!happyGlobal.Error) happyGlobal.Error = Error
if (!happyGlobal.RangeError) happyGlobal.RangeError = RangeError

const { Message, MessageContent, MessageToolbar, AssistantMessageBody } =
  await import('./message')

let container: HTMLDivElement
let root: Root

async function renderToDOM(element: ReturnType<typeof createElement>) {
  container = document.createElement('div')
  document.body.appendChild(container)
  root = createRoot(container)
  root.render(element)
  await new Promise((resolve) => setTimeout(resolve, 10))
  return container
}

function cleanupDOM() {
  try {
    root?.unmount()
  } catch {}
  if (container?.parentNode) container.parentNode.removeChild(container)
}

beforeEach(() => {
  mock.restore()
})

afterEach(() => {
  cleanupDOM()
})

describe('assistant message scrolling', () => {
  it('adds assistant-only scroll variant tokens to message content', async () => {
    const el = await renderToDOM(
      createElement(
        Message,
        { from: 'assistant' },
        createElement(MessageContent, null, 'Long assistant response'),
      ),
    )

    const content = el.querySelector('.is-assistant > div') as HTMLDivElement | null
    const tokens = (content?.className ?? '').split(/\s+/)

    expect(content).toBeTruthy()
    expect(tokens).toContain('group-[.is-assistant]:max-h-[28rem]')
    expect(tokens).toContain('group-[.is-assistant]:overflow-y-auto')
  })

  it('does not apply unconditional scroll tokens to user message content', async () => {
    const el = await renderToDOM(
      createElement(
        Message,
        { from: 'user' },
        createElement(MessageContent, null, 'User prompt'),
      ),
    )

    const content = el.querySelector('.is-user > div') as HTMLDivElement | null
    const tokens = (content?.className ?? '').split(/\s+/)

    expect(content).toBeTruthy()
    expect(tokens).not.toContain('max-h-[28rem]')
    expect(tokens).not.toContain('overflow-y-auto')
  })
})

describe('assistant scroll-body contract (Task 1)', () => {
  it('renders assistant content inside a dedicated scroll body', async () => {
    const el = await renderToDOM(
      createElement(
        Message,
        { from: 'assistant' },
        createElement(
          MessageContent,
          null,
          createElement(AssistantMessageBody, null, 'Long assistant response'),
        ),
      ),
    )

    const wrapper = el.querySelector('.is-assistant > div') as HTMLDivElement | null
    const body = wrapper?.querySelector('[data-role="assistant-scroll-body"]') as
      | HTMLDivElement
      | null

    expect(wrapper).toBeTruthy()
    expect(body).toBeTruthy()
    expect(body?.className).toContain('max-h-[min(28rem,60vh)]')
    expect(body?.className).toContain('overflow-y-auto')
    expect(body?.className).toContain('overscroll-contain')
  })

  it('keeps assistant footer content outside the scroll body', async () => {
    const el = await renderToDOM(
      createElement(
        Message,
        { from: 'assistant' },
        createElement(
          MessageContent,
          null,
          createElement(AssistantMessageBody, null, 'Scrollable body'),
          createElement(MessageToolbar, null, 'Toolbar actions'),
        ),
      ),
    )

    const body = el.querySelector('[data-role="assistant-scroll-body"]') as
      | HTMLDivElement
      | null
    const toolbar = el.querySelector('[data-role="message-toolbar"]') as
      | HTMLDivElement
      | null

    expect(body).toBeTruthy()
    expect(toolbar).toBeTruthy()
    expect(body?.contains(toolbar as Node)).toBe(false)
    expect(body?.getAttribute('tabindex')).toBe('0')
    expect(body?.getAttribute('aria-label')).toBe('Assistant message content')
  })

  it('does not apply assistant scroll-body attributes to user content', async () => {
    const el = await renderToDOM(
      createElement(
        Message,
        { from: 'user' },
        createElement(MessageContent, null, 'User prompt'),
      ),
    )

    const userBody = el.querySelector('[data-role="assistant-scroll-body"]')
    const content = el.querySelector('.is-user > div') as HTMLDivElement | null

    expect(content).toBeTruthy()
    expect(userBody).toBeNull()
    expect((content?.className ?? '').split(/\s+/)).not.toContain('overflow-y-auto')
  })
})
