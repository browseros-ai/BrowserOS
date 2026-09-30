import { afterEach, beforeEach, describe, expect, it, mock } from 'bun:test'
import { parseHTML } from 'linkedom'
import { act } from 'react'
import type { Root } from 'react-dom/client'

interface HookState {
  invitation: { eligible: boolean; bookUrl?: string; round?: number }
  invitationUpdatedAt: number
  recorded: string[]
  requests: Array<{ outcome: string; round?: number }>
  settled: { eligible: boolean; bookUrl?: string; round?: number }
  recordSucceeds: boolean
  cached: unknown[]
  tracked: string[]
  errors: string[]
  opened: string[]
  capturing: boolean
}

const state: HookState = {
  invitation: { eligible: false },
  invitationUpdatedAt: 1_000,
  recorded: [],
  requests: [],
  settled: { eligible: false },
  recordSucceeds: true,
  cached: [],
  tracked: [],
  errors: [],
  opened: [],
  capturing: true,
}

const invitationKey = ['api', 'feedback', 'invitation']

mock.module('@/modules/api/feedback.hooks', () => ({
  useFeedbackInvitation: Object.assign(
    () => ({
      data: state.invitation,
      dataUpdatedAt: state.invitationUpdatedAt,
    }),
    {
      getKey: () => invitationKey,
    },
  ),
  useRecordFeedbackInvite: () => ({
    mutate: (
      request: { outcome: string; round?: number },
      options?: {
        onSuccess?: (settled: unknown) => void
        onError?: (error: Error) => void
      },
    ) => {
      state.recorded.push(request.outcome)
      state.requests.push(request)
      if (state.recordSucceeds) options?.onSuccess?.(state.settled)
      else options?.onError?.(new Error('sidecar unavailable'))
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

const captureStateListeners = new Set<() => void>()

mock.module('@/modules/analytics/posthog', () => ({
  isCapturing: () => state.capturing,
  subscribeToCaptureState: (listener: () => void) => {
    captureStateListeners.add(listener)
    return () => captureStateListeners.delete(listener)
  },
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

mock.module('sonner', () => ({
  toast: {
    error: (message: string) => state.errors.push(message),
  },
}))

const storage: Record<string, string> = {}
Object.defineProperty(globalThis, 'localStorage', {
  configurable: true,
  value: {
    getItem: (key: string) => storage[key] ?? null,
    setItem: (key: string, value: string) => {
      storage[key] = value
    },
    removeItem: (key: string) => {
      delete storage[key]
    },
  },
})

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
  state.invitationUpdatedAt = 1_000
  state.recorded = []
  state.requests = []
  state.settled = { eligible: false }
  state.recordSucceeds = true
  state.cached = []
  state.tracked = []
  state.errors = []
  for (const key of Object.keys(storage)) delete storage[key]
  state.opened = []
  state.capturing = true

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

async function setCapturing(capturing: boolean) {
  await act(async () => {
    state.capturing = capturing
    for (const listener of captureStateListeners) listener()
  })
}

async function publishDismissal(dismissedAt: number) {
  storage['feedbackInviteDismissedAt:v1'] = String(dismissedAt)
  await act(async () => {
    const event = new window.Event('storage')
    Object.defineProperty(event, 'key', {
      configurable: true,
      value: 'feedbackInviteDismissedAt:v1',
    })
    window.dispatchEvent(event)
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

  it('stays on screen after booking', async () => {
    state.invitation = eligible
    state.settled = eligible
    await render()

    await click(buttonWithText('Book a 15 minute chat'))

    expect(container.textContent).toContain(
      "You're one of our most active users",
    )
    expect(buttonWithText('Book a 15 minute chat')).toBeDefined()
    expect(storage['feedbackInviteDismissedAt:v1']).toBeUndefined()
  })

  /// The card returns on every cockpit load until it is dismissed, and the cockpit is the
  /// new tab page. Counting every appearance would report thousands of impressions for one
  /// reader and leave the funnel without a usable denominator.
  it('counts the impression once per profile but still shows the card', async () => {
    state.invitation = eligible
    await render()
    expect(state.tracked).toEqual(['feedback_invite_shown'])
    expect(state.recorded).toEqual(['shown'])

    // A later cockpit load, same profile.
    state.tracked = []
    state.recorded = []
    await act(async () => root.unmount())
    const { createRoot } = await import('react-dom/client')
    root = createRoot(container)
    await render()

    expect(container.textContent).toContain(
      "You're one of our most active users",
    )
    expect(state.tracked).toEqual([])
    expect(state.recorded).toEqual(
      ['shown'],
      // The server is still told, because it holds the first-seen timestamp.
    )
  })

  it('counts the impression when analytics becomes ready on the same mount', async () => {
    state.invitation = eligible
    state.capturing = false
    await render()

    expect(container.textContent).toContain(
      "You're one of our most active users",
    )
    expect(state.tracked).toEqual([])
    expect(storage.feedbackInviteShownTracked).toBeUndefined()

    await setCapturing(true)

    expect(state.tracked).toEqual(['feedback_invite_shown'])
    expect(storage.feedbackInviteShownTracked).toBe('true')
  })

  /// The card returns on every load until dismissed, so a dismissal in one tab has to reach
  /// the tabs the reader already has open rather than leaving them still offering it.
  it('stays away in another tab once dismissed', async () => {
    state.invitation = eligible
    await render()
    await click(buttonWithText('No thanks'))
    expect(container.innerHTML).toBe('')

    // Another cockpit tab, same profile, whose cached answer still says eligible.
    state.tracked = []
    state.recorded = []
    await act(async () => root.unmount())
    const { createRoot } = await import('react-dom/client')
    root = createRoot(container)
    await render()

    expect(container.innerHTML).toBe('')
    expect(state.tracked).toEqual([])
    expect(state.recorded).toEqual([])
  })

  it('does not persist a browser dismissal when the server write fails', async () => {
    state.invitation = eligible
    state.recordSucceeds = false
    await render()

    await click(buttonWithText('No thanks'))

    expect(container.innerHTML).toBe('')
    expect(storage['feedbackInviteDismissedAt:v1']).toBeUndefined()
    expect(state.errors).toEqual([
      'Could not save your response. The invitation may appear again.',
    ])

    state.recorded = []
    await act(async () => root.unmount())
    const { createRoot } = await import('react-dom/client')
    root = createRoot(container)
    await render()

    expect(container.textContent).toContain(
      "You're one of our most active users",
    )
    expect(state.recorded).toEqual(['shown'])
  })

  it('lets a fresh eligible server answer override an older dismissal fence', async () => {
    state.invitation = eligible
    storage['feedbackInviteDismissedAt:v1'] = '2000'
    state.invitationUpdatedAt = 3_000

    await render()

    expect(container.textContent).toContain(
      "You're one of our most active users",
    )
  })

  it('hides an already-visible card when another tab confirms dismissal', async () => {
    state.invitation = eligible
    state.invitationUpdatedAt = 1_000
    await render()

    await publishDismissal(2_000)

    expect(container.innerHTML).toBe('')
  })

  it('reopens the link on a second click without reporting it twice', async () => {
    state.invitation = eligible
    state.settled = eligible
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
    state.settled = eligible
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

  it('reports the second round on shown, clicked, and dismissed outcomes', async () => {
    state.invitation = { ...eligible, round: 2 }
    state.settled = eligible
    await render()
    await click(buttonWithText('Book a 15 minute chat'))
    expect(buttonWithText('No thanks')).toBeDefined()
    await click(buttonWithText('No thanks'))

    expect(state.requests).toEqual([
      { outcome: 'shown', round: 2 },
      { outcome: 'clicked', round: 2 },
      { outcome: 'dismissed', round: 2 },
    ])
  })

  it('pins the shown round and booking URL when the query changes', async () => {
    state.invitation = { ...eligible, round: 1 }
    state.settled = eligible
    state.capturing = false
    await render()
    state.invitation = {
      eligible: true,
      bookUrl: 'https://cal.test/new-booking',
      round: 2,
    }
    await render()
    await setCapturing(true)
    await click(buttonWithText('Book a 15 minute chat'))
    await click(buttonWithText('No thanks'))

    expect(state.opened).toEqual([eligible.bookUrl])
    expect(state.requests).toEqual([
      { outcome: 'shown', round: 1 },
      { outcome: 'clicked', round: 1 },
      { outcome: 'dismissed', round: 1 },
    ])
    expect(storage.feedbackInviteShownTracked).toBe('true')
    expect(storage['feedbackInviteShownTracked:2']).toBeUndefined()
  })

  it('omits the round on every outcome for an older server', async () => {
    state.invitation = eligible
    state.settled = eligible
    await render()
    await click(buttonWithText('Book a 15 minute chat'))
    await click(buttonWithText('No thanks'))

    expect(state.requests).toEqual([
      { outcome: 'shown' },
      { outcome: 'clicked' },
      { outcome: 'dismissed' },
    ])
  })

  it('hides a modern invitation and fences other cached tabs after a confirmed click', async () => {
    state.invitation = { ...eligible, round: 2 }
    await render()
    await click(buttonWithText('Book a 15 minute chat'))

    expect(container.innerHTML).toBe('')
    expect(state.opened).toEqual([eligible.bookUrl])
    expect(state.requests).toEqual([
      { outcome: 'shown', round: 2 },
      { outcome: 'clicked', round: 2 },
    ])
    expect(Number(storage['feedbackInviteDismissedAt:v1'])).toBeGreaterThan(0)

    state.recorded = []
    await act(async () => root.unmount())
    const { createRoot } = await import('react-dom/client')
    root = createRoot(container)
    await render()

    expect(container.innerHTML).toBe('')
    expect(state.recorded).toEqual([])
  })

  it('hides a visible modern invitation when another tab confirms a click', async () => {
    state.invitation = { ...eligible, round: 2 }
    await render()

    await publishDismissal(2_000)

    expect(container.innerHTML).toBe('')
  })

  it('keeps a failed booking click available to retry without fencing other tabs', async () => {
    state.invitation = { ...eligible, round: 2 }
    state.recordSucceeds = false
    await render()
    await click(buttonWithText('Book a 15 minute chat'))

    expect(buttonWithText('Book a 15 minute chat')).toBeDefined()
    expect(storage['feedbackInviteDismissedAt:v1']).toBeUndefined()
    expect(state.errors).toEqual([
      'Could not record your response. Please try again.',
    ])

    state.recordSucceeds = true
    await click(buttonWithText('Book a 15 minute chat'))

    expect(state.recorded).toEqual(['shown', 'clicked', 'clicked'])
    expect(state.opened).toEqual([eligible.bookUrl, eligible.bookUrl])
    expect(container.innerHTML).toBe('')
    expect(Number(storage['feedbackInviteDismissedAt:v1'])).toBeGreaterThan(0)
  })

  it('does not hide or fence the card when a click reply remains eligible', async () => {
    state.invitation = { ...eligible, round: 1 }
    state.settled = { ...eligible, round: 2 }
    await render()
    await click(buttonWithText('Book a 15 minute chat'))

    expect(buttonWithText('Book a 15 minute chat')).toBeDefined()
    expect(storage['feedbackInviteDismissedAt:v1']).toBeUndefined()
    expect(state.cached).toEqual([{ key: invitationKey, value: state.settled }])
  })

  it('does not fence another tab when a stale dismissal returns a newer round', async () => {
    state.invitation = { ...eligible, round: 1 }
    state.settled = { ...eligible, round: 2 }
    await render()
    await click(buttonWithText('No thanks'))

    expect(container.innerHTML).toBe('')
    expect(storage['feedbackInviteDismissedAt:v1']).toBeUndefined()
    expect(state.cached).toEqual([{ key: invitationKey, value: state.settled }])

    await act(async () => root.unmount())
    const { createRoot } = await import('react-dom/client')
    root = createRoot(container)
    state.invitation = state.settled
    await render()

    expect(buttonWithText('Book a 15 minute chat')).toBeDefined()
    expect(state.requests.at(-1)).toEqual({ outcome: 'shown', round: 2 })
  })

  it('honors the legacy impression key for round one without hiding the card', async () => {
    storage.feedbackInviteShownTracked = 'true'
    state.invitation = { ...eligible, round: 1 }
    await render()

    expect(state.tracked).toEqual([])
    expect(state.requests).toEqual([{ outcome: 'shown', round: 1 }])
    expect(buttonWithText('Book a 15 minute chat')).toBeDefined()
  })

  it('deduplicates impressions per round without spending rounds on reload', async () => {
    storage.feedbackInviteShownTracked = 'true'
    for (const round of [2, 2, 3, 3]) {
      state.invitation = { ...eligible, round }
      await render()
      expect(buttonWithText('Book a 15 minute chat')).toBeDefined()
      await act(async () => root.unmount())
      const { createRoot } = await import('react-dom/client')
      root = createRoot(container)
    }

    expect(state.tracked).toEqual([
      'feedback_invite_shown',
      'feedback_invite_shown',
    ])
    expect(state.requests).toEqual([
      { outcome: 'shown', round: 2 },
      { outcome: 'shown', round: 2 },
      { outcome: 'shown', round: 3 },
      { outcome: 'shown', round: 3 },
    ])
  })
})
