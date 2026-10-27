import { afterEach, beforeEach, describe, expect, it, mock } from 'bun:test'
import type { SessionSummary } from '@browseros/claw-api'
import { parseHTML } from 'linkedom'
import { act } from 'react'
import type { Root } from 'react-dom/client'
import type { ReplaySession } from '@/screens/replay/use-replay-session'

let panelSession: unknown

mock.module('@/screens/replay/ReplayPanel', () => ({
  ReplayPanel: ({
    session,
    fullScreen = false,
    onToggleFullScreen,
  }: {
    session: ReplaySession
    fullScreen?: boolean
    onToggleFullScreen?: () => void
  }) => {
    panelSession = session
    return (
      <div data-replay-panel data-full-screen={fullScreen}>
        <button type="button" data-exit onClick={onToggleFullScreen}>
          Exit full screen
        </button>
      </div>
    )
  },
}))

const { ReplayTheater } = await import('./ReplayTheater')

const session: SessionSummary = {
  sessionId: 'session-1',
  slug: 'outreach',
  label: 'outreach',
  name: 'Browsed www.linkedin.com',
  startedAt: Date.now(),
  durationMs: 6 * 60_000,
  dispatchCount: 7,
  toolSequence: [],
  status: 'done',
  errorCount: 0,
}
// The theater hands the session to the player untouched; nothing reads it here.
const replay = {
  marker: 'the page’s replay session',
} as unknown as ReplaySession

const globalDescriptors = new Map(
  [
    'window',
    'document',
    'navigator',
    'HTMLElement',
    'Element',
    'Node',
    'Event',
    'KeyboardEvent',
  ].map((name) => [name, Object.getOwnPropertyDescriptor(globalThis, name)]),
)

let root: Root
let container: HTMLElement
let changes: boolean[]

function Harness({ open }: { open: boolean }) {
  return (
    <>
      <input data-text-field />
      <div role="dialog" data-other-dialog>
        <button type="button">Inside another dialog</button>
      </div>
      <ReplayTheater
        open={open}
        onOpenChange={(next) => changes.push(next)}
        session={session}
        replay={replay}
        steps={<ol data-steps-rail />}
      />
    </>
  )
}

beforeEach(async () => {
  changes = []
  panelSession = undefined
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
    KeyboardEvent: dom.window.KeyboardEvent ?? dom.window.Event,
  }
  for (const [name, value] of Object.entries(globals)) {
    Object.defineProperty(globalThis, name, {
      configurable: true,
      writable: true,
      value,
    })
  }
  Object.assign(dom.window, {
    requestAnimationFrame: () => 1,
    cancelAnimationFrame: () => {},
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

/** A keydown from `target`, bubbling the way a real key press does. */
function press(
  target: EventTarget,
  key: string,
  modifiers: Partial<Record<'metaKey' | 'ctrlKey' | 'altKey', boolean>> = {},
) {
  const event = new window.Event('keydown', {
    bubbles: true,
    cancelable: true,
  }) as KeyboardEvent
  Object.assign(event, { key, ...modifiers })
  target.dispatchEvent(event)
  return event
}

const dialog = () =>
  document.querySelector('[role="dialog"][aria-modal="true"]')

describe('ReplayTheater', () => {
  it('opens on F from the page', async () => {
    await act(async () => root.render(<Harness open={false} />))
    expect(dialog()).toBeNull()

    const event = press(document.body, 'f')
    expect(changes).toEqual([true])
    expect(event.defaultPrevented).toBe(true)

    press(document.body, 'F')
    expect(changes).toEqual([true, true])
  })

  it('leaves F alone when it is typed, chorded, or meant for another dialog', async () => {
    await act(async () => root.render(<Harness open={false} />))

    press(document.body, 'f', { metaKey: true })
    press(document.body, 'f', { ctrlKey: true })
    press(document.body, 'f', { altKey: true })
    press(container.querySelector('[data-text-field]') as Element, 'f')
    press(container.querySelector('[data-other-dialog] button') as Element, 'f')
    expect(changes).toEqual([])
  })

  it('shows the task and its steps beside the shared player', async () => {
    await act(async () => root.render(<Harness open />))

    const theater = dialog()
    expect(theater).not.toBeNull()
    const titleId = theater?.getAttribute('aria-labelledby') ?? ''
    expect(document.getElementById(titleId)?.textContent).toBe(
      'Browsed www.linkedin.com',
    )
    expect(theater?.textContent).toContain('Done')
    expect(theater?.textContent).toContain('outreach')
    expect(theater?.textContent).toContain('took 6 min')
    expect(theater?.querySelector('[data-steps-rail]')).not.toBeNull()
    expect(
      theater
        ?.querySelector('[data-replay-panel]')
        ?.getAttribute('data-full-screen'),
    ).toBe('true')
    expect(panelSession).toBe(replay)
    // Task content stays out of session recordings outside the page wrapper.
    expect(theater?.className).toContain('ph-no-capture')
  })

  it('closes from its close button, the player, and Escape', async () => {
    await act(async () => root.render(<Harness open />))
    const click = (selector: string) =>
      dialog()
        ?.querySelector(selector)
        ?.dispatchEvent(new window.Event('click', { bubbles: true }))

    await act(async () => click('button[aria-label="Close full screen"]'))
    await act(async () => click('[data-exit]'))
    await act(async () => {
      press(dialog() as Element, 'Escape')
    })
    expect(changes).toEqual([false, false, false])

    // F only opens; while open it is just a key.
    press(dialog() as Element, 'f')
    expect(changes).toEqual([false, false, false])
  })
})
