import { afterEach, beforeEach, describe, expect, it, mock } from 'bun:test'
import { parseHTML } from 'linkedom'
import { act } from 'react'
import type { Root } from 'react-dom/client'

interface HookState {
  invitation: { eligible: boolean; bookUrl?: string }
  recorded: string[]
  cached: unknown[]
  tracked: string[]
  opened: string[]
}

const state: HookState = {
  invitation: { eligible: false },
  recorded: [],
  cached: [],
  tracked: [],
  opened: [],
}

const invitationKey = ['api', 'feedback', 'invitation']

mock.module('@/modules/api/feedback.hooks', () => ({
  useFeedbackInvitation: Object.assign(() => ({ data: state.invitation }), {
    getKey: () => invitationKey,
  }),
  useRecordFeedbackInvite: () => ({
    mutate: (
      { outcome }: { outcome: string },
      options?: { onSuccess?: (settled: unknown) => void },
    ) => {
      state.recorded.push(outcome)
      options?.onSuccess?.({ eligible: false })
    },
  }),
}))

mock.module('@tanstack/react-query', () => ({
  useQueryClient: () => ({
    setQueryData: (key: unknown, value: unknown) => {
      state.cached.push({ key, value })
    },
  }),
}))

mock.module('@/modules/analytics/events', () => ({
  AnalyticsEvent: {
    FeedbackInviteShown: 'feedback_invite_shown',
    FeedbackInviteClicked: 'feedback_invite_clicked',
    FeedbackInviteDismissed: 'feedback_invite_dismissed',
  },
  track: (event: string) => {
    state.tracked.push(event)
  },
}))

const globalDescriptors = new Map(
  ['window', 'document', 'navigator', 'HTMLElement', 'Node', 'Event'].map(
    (name) => [name, Object.getOwnPropertyDescriptor(globalThis, name)],
  ),
)

const { FeedbackInviteCard } = await import('./FeedbackInviteCard')

let root: Root
let container: HTMLElement

beforeEach(async () => {
  state.invitation = { eligible: false }
  state.recorded = []
  state.cached = []
  state.tracked = []
  state.opened = []

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
  Object.defineProperty(globalThis, 'chrome', {
    configurable: true,
    writable: true,
    value: {
      tabs: {
        create: ({ url }: { url: string }) => {
          state.opened.push(url)
        },
      },
    },
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
  Reflect.deleteProperty(globalThis, 'chrome')
})

async function render() {
  await act(async () => {
    root.render(<FeedbackInviteCard />)
  })
}

function buttonWithText(text: string): HTMLElement {
  const match = [...container.querySelectorAll('button')].find(
    (button) => (button.textContent ?? '').trim() === text,
  )
  if (!match) throw new Error(`no button "${text}"`)
  return match as unknown as HTMLElement
}

function dismissControl(): HTMLElement {
  const match = container.querySelector('button[aria-label="Dismiss"]')
  if (!match) throw new Error('no dismiss control')
  return match as unknown as HTMLElement
}

async function click(element: HTMLElement) {
  await act(async () => {
    element.dispatchEvent(new window.Event('click', { bubbles: true }))
  })
}

const eligible = { eligible: true, bookUrl: 'https://cal.test/book' }

describe('FeedbackInviteCard', () => {
  it('renders nothing for an installation the server has not invited', async () => {
    await render()

    expect(container.innerHTML).toBe('')
    expect(state.recorded).toEqual([])
    expect(state.tracked).toEqual([])
  })

  it('shows the invitation and records the impression once', async () => {
    state.invitation = eligible
    await render()

    expect(container.textContent).toContain(
      "You're one of our most active users",
    )
    expect(state.recorded).toEqual(['shown'])
    expect(state.tracked).toEqual(['feedback_invite_shown'])

    // A re-render must not report a second impression for the same card.
    await render()
    expect(state.recorded).toEqual(['shown'])
  })

  /// Recording the impression is what makes the server answer 'not eligible', so a card
  /// that followed the query would erase itself the moment it appeared.
  it('stays on screen after the impression makes the query ineligible', async () => {
    state.invitation = eligible
    await render()

    state.invitation = { eligible: false }
    await render()

    expect(container.textContent).toContain(
      "You're one of our most active users",
    )
  })

  it('opens the link the server supplied and reports the click', async () => {
    state.invitation = eligible
    await render()

    await click(buttonWithText('Book a 15 minute chat'))

    expect(state.opened).toEqual(['https://cal.test/book'])
    expect(state.recorded).toEqual(['shown', 'clicked'])
    expect(state.tracked).toContain('feedback_invite_clicked')
    // The answer to a recorded outcome is the invitation's new state, so it
    // lands in the cache rather than leaving a stale eligible entry behind.
    expect(state.cached).toEqual([
      { key: invitationKey, value: { eligible: false } },
    ])
  })

  /// Taking the card away on booking would punish the reader for accepting: they
  /// land on a booking page, and if they come back to finish later the invitation
  /// they agreed to has gone. Only declining removes it.
  it('stays on screen after booking', async () => {
    state.invitation = eligible
    await render()

    await click(buttonWithText('Book a 15 minute chat'))

    expect(container.textContent).toContain(
      "You're one of our most active users",
    )
    expect(buttonWithText('Book a 15 minute chat')).toBeDefined()
  })

  it('reopens the link on a second click without reporting it twice', async () => {
    state.invitation = eligible
    await render()

    await click(buttonWithText('Book a 15 minute chat'))
    await click(buttonWithText('Book a 15 minute chat'))

    expect(state.opened).toEqual([
      'https://cal.test/book',
      'https://cal.test/book',
    ])
    expect(state.recorded).toEqual(['shown', 'clicked'])
    expect(
      state.tracked.filter((event) => event === 'feedback_invite_clicked'),
    ).toHaveLength(1)
  })

  it('can still be dismissed after booking', async () => {
    state.invitation = eligible
    await render()

    await click(buttonWithText('Book a 15 minute chat'))
    await click(buttonWithText('No thanks'))

    expect(state.recorded).toEqual(['shown', 'clicked', 'dismissed'])
    expect(container.innerHTML).toBe('')
  })

  it('reports a dismissal from the worded control and removes the card', async () => {
    state.invitation = eligible
    await render()

    await click(buttonWithText('No thanks'))

    expect(state.recorded).toEqual(['shown', 'dismissed'])
    expect(state.tracked).toContain('feedback_invite_dismissed')
    expect(container.innerHTML).toBe('')
    expect(state.opened).toEqual([])
  })

  it('treats the dismiss control the same as declining in words', async () => {
    state.invitation = eligible
    await render()

    await click(dismissControl())

    expect(state.recorded).toEqual(['shown', 'dismissed'])
    expect(container.innerHTML).toBe('')
  })
})
