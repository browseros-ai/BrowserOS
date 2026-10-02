import { afterEach, beforeEach, describe, expect, it, mock } from 'bun:test'
import type { HelpRequest, SessionBrowserTab } from '@browseros/claw-api'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { parseHTML } from 'linkedom'
import { act, type ComponentProps, type ReactNode } from 'react'
import type { Root } from 'react-dom/client'
import * as _popover from '@/components/ui/popover'
import * as _auditHooks from '@/modules/api/audit.hooks'
import * as _cancelHooks from '@/modules/api/cancel.hooks'
import * as _focusHooks from '@/modules/api/focus.hooks'
import * as _helpHooks from '@/modules/api/help.hooks'
import type { LiveSessionCardRecord } from './cockpit.helpers'

const focusCalls: Array<{ browserTabId: number }> = []
const resolveCalls: Array<{ sessionId: string; note?: string }> = []
let resolveResult = { resolved: true }
let focusShouldFail = false

mock.module('@/modules/api/audit.hooks', () => ({
  ..._auditHooks,
  useSessionPreviewUrl: () => null,
  // Null base keeps LivePreview from opening a real EventSource under linkedom.
  useApiBaseUrl: () => null,
}))

mock.module('@/modules/api/cancel.hooks', () => ({
  ..._cancelHooks,
  useCancelSession: () => ({
    isPending: false,
    variables: undefined,
    mutate: (
      _variables: { sessionId: string },
      options?: { onSuccess?: () => void },
    ) => options?.onSuccess?.(),
  }),
}))

mock.module('@/modules/api/focus.hooks', () => ({
  ..._focusHooks,
  useFocusBrowserTab: () => ({
    isPending: false,
    variables: undefined,
    mutate: (
      variables: { browserTabId: number },
      options?: { onSettled?: () => void; onError?: (err: Error) => void },
    ) => {
      focusCalls.push(variables)
      if (focusShouldFail) options?.onError?.(new Error('tab gone'))
      options?.onSettled?.()
    },
  }),
}))

mock.module('@/modules/api/help.hooks', () => ({
  ..._helpHooks,
  useResolveHelp: () => ({
    isPending: false,
    variables: undefined,
    mutate: (
      variables: { sessionId: string; note?: string },
      options?: { onSuccess?: (result: { resolved: boolean }) => void },
    ) => {
      resolveCalls.push(variables)
      options?.onSuccess?.(resolveResult)
    },
  }),
}))

mock.module('sonner', () => ({
  toast: Object.assign(() => {}, {
    success: () => {},
    error: () => {},
    message: () => {},
  }),
}))

mock.module('@/components/ui/popover', () => ({
  ..._popover,
  Popover: ({ children }: { children?: ReactNode }) => <>{children}</>,
  PopoverTrigger: (props: ComponentProps<'button'>) => <button {...props} />,
  PopoverContent: (props: ComponentProps<'div'>) => (
    <div data-slot="popover-content" {...props} />
  ),
}))

const globalDescriptors = new Map(
  ['window', 'document', 'navigator', 'HTMLElement', 'Node', 'Event'].map(
    (name) => [name, Object.getOwnPropertyDescriptor(globalThis, name)],
  ),
)

const { RunningGrid } = await import('@/components/cockpit/RunningGrid')
const { NeedsYouBanner } = await import('@/components/cockpit/NeedsYouBanner')
const { InControlBar } = await import('@/components/cockpit/InControlBar')
const { useHelpTakeover } = await import('./cockpit.hooks')

function helpRequest(over: Partial<HelpRequest> = {}): HelpRequest {
  return {
    requestId: 'help-1',
    reason: 'Enter the LinkedIn verification code',
    resumeHint: 'resume at profile 15 of 22',
    browserTabId: 77,
    url: 'https://www.linkedin.com/checkpoint',
    requestedAt: 1_000,
    ...over,
  }
}

function browserTab(over: Partial<SessionBrowserTab> = {}): SessionBrowserTab {
  return {
    browserTabId: 77,
    url: 'https://www.linkedin.com/checkpoint',
    title: 'Security verification',
    firstActivityAt: 1_000,
    lastActivityAt: 1_000,
    lastToolName: 'navigate',
    toolCount: 1,
    recentTools: [{ name: 'navigate', at: 1_000 }],
    ...over,
  }
}

function session(
  over: Partial<LiveSessionCardRecord> = {},
): LiveSessionCardRecord {
  const tab = browserTab()
  return {
    sessionId: 'session-blocked',
    profileId: 'profile-shared',
    slug: 'claude-code',
    label: 'Claude-code',
    name: 'Outreach messaging',
    color: '#0254ec',
    startedAt: 100,
    state: 'active',
    selectedTab: tab,
    browserTabs: [tab],
    toolCount: 1,
    recentTools: [{ name: 'navigate', at: 1_000 }],
    helpRequest: helpRequest(),
    ...over,
  }
}

function MiniCockpit({ sessions }: { sessions: LiveSessionCardRecord[] }) {
  const takeover = useHelpTakeover(sessions)
  return (
    <>
      <NeedsYouBanner
        sessions={sessions}
        onTakeOver={takeover.takeOver}
        onStop={takeover.stop}
        pendingTakeOverSessionId={takeover.pendingTakeOverSessionId}
        cancelPendingSessionId={takeover.stopPendingSessionId}
      />
      <RunningGrid
        sessions={sessions}
        onTakeOver={takeover.takeOver}
        takeOverPendingSessionId={takeover.pendingTakeOverSessionId}
      />
      {takeover.inControlSession && (
        <InControlBar
          session={takeover.inControlSession}
          onHandBack={takeover.handBack}
          onCancel={takeover.cancelControl}
          isHandingBack={takeover.isHandingBack}
        />
      )}
    </>
  )
}

let root: Root
let container: HTMLElement
let queryClient: QueryClient

beforeEach(async () => {
  focusCalls.length = 0
  resolveCalls.length = 0
  resolveResult = { resolved: true }
  focusShouldFail = false
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
  queryClient = new QueryClient()
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

async function render(sessions: LiveSessionCardRecord[]) {
  await act(async () =>
    root.render(
      <QueryClientProvider client={queryClient}>
        <MiniCockpit sessions={sessions} />
      </QueryClientProvider>,
    ),
  )
}

function click(selector: string) {
  const el = container.querySelector(selector)
  if (!el) throw new Error(`missing element: ${selector}`)
  return act(async () => {
    el.dispatchEvent(new window.Event('click', { bubbles: true }))
  })
}

describe('cockpit human-help takeover', () => {
  it('flips a blocked session to the needs-you state with its reason and timer', async () => {
    await render([session()])

    const card = container.querySelector(
      '[data-session-card="session-blocked"]',
    )
    expect(
      card?.querySelector('[data-caption-tone="needs-you"]'),
    ).not.toBeNull()
    expect(card?.querySelector('[data-caption-tone="blue"]')).toBeNull()
    expect(card?.textContent).toContain('Enter the LinkedIn verification code')
    expect(card?.textContent).toContain('Needs you')
    // The prominent banner also surfaces the request.
    expect(container.querySelector('[data-needs-you-banner]')).not.toBeNull()
  })

  it('foregrounds the blocked tab and opens the in-control bar on Take over', async () => {
    await render([session()])

    expect(container.querySelector('[data-hand-back]')).toBeNull()
    await click('[data-session-card="session-blocked"] [data-take-over]')

    expect(focusCalls).toEqual([{ browserTabId: 77 }])
    expect(
      container.querySelector('[data-hand-back="session-blocked"]'),
    ).not.toBeNull()
  })

  it('resolves the session and closes the bar on Hand back', async () => {
    await render([session()])
    await click('[data-session-card="session-blocked"] [data-take-over]')

    // The bar carries the resume hint and a note affordance for the agent.
    const bar = container.querySelector('[data-hand-back]')?.closest('div')
    expect(container.textContent).toContain('resume at profile 15 of 22')
    expect(container.querySelector('#hand-back-note')).not.toBeNull()
    expect(bar).not.toBeNull()

    await click('[data-hand-back="session-blocked"]')

    expect(resolveCalls).toHaveLength(1)
    expect(resolveCalls[0]?.sessionId).toBe('session-blocked')
    expect(container.querySelector('[data-hand-back]')).toBeNull()
  })

  it('does not open the in-control bar when focusing the tab fails', async () => {
    focusShouldFail = true
    await render([session()])

    await click('[data-session-card="session-blocked"] [data-take-over]')

    // The tab is gone, so there is nothing to take over and no Hand back.
    expect(container.querySelector('[data-hand-back]')).toBeNull()
  })

  it('hides the banner and cards when no session is waiting', async () => {
    await render([session({ helpRequest: undefined })])

    expect(container.querySelector('[data-needs-you-banner]')).toBeNull()
    expect(
      container.querySelector('[data-caption-tone="needs-you"]'),
    ).toBeNull()
    expect(container.querySelector('[data-caption-tone="blue"]')).not.toBeNull()
  })
})
