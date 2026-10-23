import { describe, expect, it } from 'bun:test'
import { renderToStaticMarkup } from 'react-dom/server'
import { MemoryRouter } from 'react-router'
import {
  BROWSEROS_ONBOARDING_API_VERSION,
  type BrowserOSOnboardingState,
} from './browseros-onboarding-api'
import { ChatThread } from './components/ChatThread'
import { flightSceneFrameAt } from './components/FlightScene'
import { ReplyDock } from './components/ReplyDock'
import { MOCK_BROWSEROS_IMPORT_SOURCES } from './mock-import-sources'
import { OnboardingChat, startImportFromClick } from './OnboardingChat'
import {
  buildOnboardingChat,
  type OnboardingChat as Chat,
  type ChatChoices,
  initialChatChoices,
} from './onboarding-chat'
import { revealPlan } from './use-chat-reveal'

function renderThread(chat: Chat): string {
  const revealed = new Set(
    chat.turns.flatMap((turn) => turn.bubbles.map((bubble) => bubble.id)),
  )
  return renderToStaticMarkup(
    <ChatThread
      turns={chat.turns}
      reveal={{ revealed, typingTurnId: null, settled: true }}
      onToggleItem={() => undefined}
    />,
  )
}

function renderDock(chat: Chat): string {
  return renderToStaticMarkup(
    <ReplyDock
      items={chat.dock}
      visible
      onAction={() => undefined}
      onPickSource={() => undefined}
    />,
  )
}

function build(
  overrides: Partial<BrowserOSOnboardingState>,
  choices: ChatChoices,
): Chat {
  return buildOnboardingChat({
    state: {
      apiVersion: BROWSEROS_ONBOARDING_API_VERSION,
      status: 'ready',
      sources: [MOCK_BROWSEROS_IMPORT_SOURCES[0]],
      ...overrides,
    },
    choices,
    platform: 'mac',
  })
}

describe('OnboardingChat page', () => {
  it('opens with the progress strip and neo typing before the first line', () => {
    const html = renderToStaticMarkup(
      <MemoryRouter>
        <OnboardingChat />
      </MemoryRouter>,
    )
    expect(html).toContain('aria-label="Onboarding progress"')
    expect(html).toContain('aria-valuenow="1"')
    expect(html).toContain('<main')
    // Nothing lands before the reveal timers run, so no reply is clickable yet.
    expect(html).not.toContain('Set me up')
  })
})

describe('ChatThread', () => {
  it('renders the Hi turn with the hero line and the flight scene', () => {
    const html = renderThread(build({}, initialChatChoices))
    expect(html).toContain('Hi, I’m BrowserOS')
    expect(html).toContain('>neo</em>')
    expect(html).toContain('I’m the browser your AI uses.')
    expect(html).toContain('google.com/travel/flights')
    // The card shows Claude Code in a terminal (owner, round 9).
    expect(html).toContain('Claude Code')
    expect(html).toContain(
      'Use neo, find me the cheapest one-way flight to NYC next Friday on Google Flights.',
    )
    expect(html).toContain('Cheapest is $142, nonstop, Fri 7:05 am.')
  })

  it('renders the chips as toggle buttons from the source', () => {
    const html = renderThread(
      build({}, { ...initialChatChoices, step: 'logins' }),
    )
    expect(html).toContain('I’ll bring these from Chrome:')
    expect(html).toContain('aria-pressed="true"')
    expect(html).toContain('Signed-in sites')
    expect(html).toMatch(/aria-pressed="false"[^>]*>.*Extensions/)
  })

  it('draws the Keychain heads-up as an inert picture', () => {
    const html = renderThread(
      build(
        { status: 'importing' },
        { ...initialChatChoices, step: 'logins', importRequested: true },
      ),
    )
    expect(html).toContain('Your Mac will ask for its password.')
    expect(html).toContain('Chrome Safe Storage')
    expect(html).not.toContain('<input')
  })
})

describe('ReplyDock', () => {
  const logins = { ...initialChatChoices, step: 'logins' as const }

  it('shows one profile as a plain chip with its display name and no chevron', () => {
    const html = renderDock(build({}, logins))
    expect(html).toContain('Google Chrome - Work')
    expect(html).not.toContain('aria-haspopup')
    expect(html).toContain('Bring my logins')
  })

  it('carries the full display name as a tooltip on the chip', () => {
    const html = renderDock(
      build({ sources: [...MOCK_BROWSEROS_IMPORT_SOURCES] }, logins),
    )
    expect(html).toContain('title="Google Chrome - Work"')
  })

  it('makes the chip a picker when there are several profiles', () => {
    const html = renderDock(
      build({ sources: [...MOCK_BROWSEROS_IMPORT_SOURCES] }, logins),
    )
    expect(html).toContain('aria-haspopup="listbox"')
    expect(html).toContain('aria-expanded="false"')
  })

  it('renders nothing until the thread has settled', () => {
    const html = renderToStaticMarkup(
      <ReplyDock
        items={build({}, initialChatChoices).dock}
        visible={false}
        onAction={() => undefined}
        onPickSource={() => undefined}
      />,
    )
    expect(html).not.toContain('Set me up')
  })
})

describe('flightSceneFrameAt', () => {
  it('asks, works through the rows, then tags the cheapest flight', () => {
    expect(flightSceneFrameAt(0)).toMatchObject({
      asked: false,
      status: 'idle',
    })
    expect(flightSceneFrameAt(3000)).toMatchObject({
      asked: true,
      status: 'working',
      ring: 0,
      checked: 1,
    })
    expect(flightSceneFrameAt(6100)).toMatchObject({ ring: 2, tagged: true })
    expect(flightSceneFrameAt(7000)).toMatchObject({
      status: 'done',
      ring: -1,
      checked: 5,
    })
  })

  it('stops the working counter when the answer lands', () => {
    expect(flightSceneFrameAt(6800).seconds).toBe(6)
    expect(flightSceneFrameAt(9800).seconds).toBe(6)
  })
})

describe('the import click', () => {
  it('commits the reply and heads-up, then sends startImport on the same stack', () => {
    const calls: string[] = []
    const request = { sourceId: 'chrome-work', items: ['cookies' as const] }
    startImportFromClick(
      request,
      { startImport: (sent) => calls.push(`startImport:${sent.sourceId}`) },
      () => calls.push('commit'),
    )
    // Synchronous: no await or timer between the click and the request.
    expect(calls).toEqual(['commit', 'startImport:chrome-work'])
  })
})

describe('revealPlan', () => {
  const requested = {
    ...initialChatChoices,
    step: 'logins' as const,
    importRequested: true,
  }
  const turns = build({}, requested).turns
  const typedBeforeClick = new Set(
    turns
      .flatMap((turn) => turn.bubbles.map((bubble) => bubble.id))
      .filter(
        (id) => !id.startsWith('you-import') && !id.startsWith('import-'),
      ),
  )

  it('shows the reply and the heads-up in the render that adds them', () => {
    const { shown, next } = revealPlan(turns, typedBeforeClick, false)
    expect(shown.has('you-import')).toBe(true)
    expect(shown.has('import-keychain')).toBe(true)
    expect(next?.id).toBe('import-progress')
  })

  it('holds instant bubbles behind one still being typed', () => {
    const { shown, next } = revealPlan(turns, new Set(['hi-hero']), false)
    expect(next?.id).toBe('hi-line')
    expect(shown.has('you-start')).toBe(false)
    expect(shown.has('import-keychain')).toBe(false)
  })

  it('shows everything at once with reduced motion', () => {
    const { shown, next } = revealPlan(turns, new Set(), true)
    expect(next).toBeNull()
    expect(shown.size).toBe(turns.flatMap((turn) => turn.bubbles).length)
  })
})
