import { describe, expect, it } from 'bun:test'
import {
  BROWSEROS_ONBOARDING_API_VERSION,
  type BrowserOSImportSource,
  type BrowserOSOnboardingState,
} from './browseros-onboarding-api'
import type { ClientPlatform } from './import-selection'
import { MOCK_BROWSEROS_IMPORT_SOURCES } from './mock-import-sources'
import {
  buildOnboardingChat,
  type ChatBubble,
  type ChatChoices,
  chatChoicesReducer,
  chatImportRequest,
  initialChatChoices,
  type OnboardingChat,
} from './onboarding-chat'

const [WORK, PERSONAL] = MOCK_BROWSEROS_IMPORT_SOURCES
const WINDOWS_STYLE_SOURCE: BrowserOSImportSource = {
  ...PERSONAL,
  id: 'source-0',
  supportedItems: ['history', 'bookmarks', 'autofill', 'extensions'],
  recommendedItems: ['history', 'bookmarks', 'autofill', 'extensions'],
}

function snapshot(
  overrides: Partial<BrowserOSOnboardingState> = {},
): BrowserOSOnboardingState {
  return {
    apiVersion: BROWSEROS_ONBOARDING_API_VERSION,
    status: 'ready',
    sources: [WORK],
    ...overrides,
  }
}

const onLogins: ChatChoices = { ...initialChatChoices, step: 'logins' }
const requested: ChatChoices = { ...onLogins, importRequested: true }

function chat(
  state: BrowserOSOnboardingState,
  choices: ChatChoices = onLogins,
  platform: ClientPlatform = 'mac',
): OnboardingChat {
  return buildOnboardingChat({ state, choices, platform })
}

function bubbles(result: OnboardingChat): ChatBubble[] {
  return result.turns.flatMap((turn) => turn.bubbles)
}

function texts(result: OnboardingChat): string[] {
  return bubbles(result).flatMap((bubble) =>
    'text' in bubble ? [bubble.text] : [],
  )
}

function dockLabels(result: OnboardingChat): string[] {
  return result.dock.map((item) =>
    item.kind === 'profile' ? `profile:${item.selectedId}` : item.label,
  )
}

function chipsOf(result: OnboardingChat) {
  const chips = bubbles(result).find((bubble) => bubble.kind === 'chips')
  if (chips?.kind !== 'chips') throw new Error('no chips bubble')
  return chips
}

describe('Hi', () => {
  it('opens with neo, the live flight scene and the two replies', () => {
    const result = chat(snapshot(), initialChatChoices)
    expect(result.progress).toBe(1)
    expect(result.turns).toHaveLength(1)
    expect(bubbles(result).map((bubble) => bubble.kind)).toEqual([
      'hero',
      'text',
      'scene',
    ])
    expect(texts(result)).toEqual(['I’m the browser your AI uses.'])
    expect(bubbles(result)[2]).toMatchObject({ kind: 'scene', live: true })
    expect(dockLabels(result)).toEqual(['Reconnect', 'Set me up'])
  })

  it('freezes the scene once the chat moves on', () => {
    const result = chat(snapshot())
    expect(bubbles(result)[2]).toMatchObject({ kind: 'scene', live: false })
  })
})

describe('Logins', () => {
  it('asks about logins with one chip per supported item, opt-ins off', () => {
    const result = chat(snapshot())
    expect(result.progress).toBe(2)
    expect(texts(result)).toContain('Want your AI to use your logins?')
    const chips = chipsOf(result)
    expect(chips.browser).toBe('Chrome')
    expect(chips.editable).toBe(true)
    expect(chips.chips.map((chip) => [chip.label, chip.on])).toEqual([
      ['Signed-in sites', true],
      ['Passwords', true],
      ['Bookmarks', true],
      ['History', true],
      ['Autofill', true],
      ['Extensions', false],
      ['Search engines', false],
    ])
    expect(dockLabels(result)).toEqual([
      'Not now',
      'profile:chrome-work',
      'Bring my logins',
    ])
  })

  it('offers the plainer ask when the source cannot bring logins', () => {
    const result = chat(snapshot({ sources: [WINDOWS_STYLE_SOURCE] }))
    expect(texts(result)).toContain('Want your Chrome things here?')
    expect(chipsOf(result).chips.map((chip) => chip.label)).toEqual([
      'Bookmarks',
      'History',
      'Autofill',
      'Extensions',
    ])
    expect(dockLabels(result).at(-1)).toBe('Bring them over')
  })

  it('switches the primary to "Bring them over" when logins are toggled off', () => {
    let choices = onLogins
    for (const item of ['cookies', 'passwords'] as const) {
      choices = chatChoicesReducer(choices, {
        type: 'toggleItem',
        source: WORK,
        item,
      })
    }
    expect(dockLabels(chat(snapshot(), choices)).at(-1)).toBe('Bring them over')
  })

  it('disables the primary when every chip is off', () => {
    let choices = onLogins
    for (const item of WORK.supportedItems.filter(
      (item) => item !== 'extensions' && item !== 'searchEngines',
    )) {
      choices = chatChoicesReducer(choices, {
        type: 'toggleItem',
        source: WORK,
        item,
      })
    }
    const primary = chat(snapshot(), choices).dock.at(-1)
    expect(primary).toMatchObject({ label: 'Bring them over', disabled: true })
  })

  it('names the picked source and resets its chips to that source’s defaults', () => {
    const toggled = chatChoicesReducer(onLogins, {
      type: 'toggleItem',
      source: WORK,
      item: 'extensions',
    })
    const picked = chatChoicesReducer(toggled, {
      type: 'pickSource',
      sourceId: PERSONAL.id,
    })
    const result = chat(snapshot({ sources: [WORK, PERSONAL] }), picked)
    expect(dockLabels(result)).toContain(`profile:${PERSONAL.id}`)
    expect(
      chipsOf(result)
        .chips.filter((chip) => chip.on)
        .map((chip) => chip.item),
    ).toEqual(['cookies', 'passwords', 'bookmarks', 'history', 'autofill'])
  })

  it('shows neo typing while Chromium is still looking for browsers', () => {
    const result = chat(snapshot({ status: 'detecting', sources: [] }))
    expect(result.turns.at(-1)).toMatchObject({ pending: true, bubbles: [] })
    expect(result.dock).toEqual([])
  })

  it('lets the person continue when no browser was found', () => {
    const result = chat(snapshot({ sources: [] }))
    expect(texts(result)).toContain(
      'I didn’t find another browser to bring things from.',
    )
    expect(dockLabels(result)).toEqual(['Look again', 'Continue'])
  })
})

describe('Import', () => {
  it('shows the heads-up as soon as the import is requested, before native answers', () => {
    // The click commits this render before it sends the request.
    const result = chat(snapshot(), requested)
    expect(
      bubbles(result)
        .map((bubble) => bubble.kind)
        .slice(-2),
    ).toEqual(['keychain', 'progress'])
    expect(texts(result).at(-1)).toBe('Bringing your logins… 0 of 5')
    expect(chipsOf(result).editable).toBe(false)
    expect(result.dock).toEqual([])
  })

  it('claims nothing specific when a success snapshot carries no progress', () => {
    const result = chat(snapshot({ status: 'succeeded' }), requested)
    expect(texts(result).at(-1)).toBe('Done.')
    expect(texts(result)).not.toContain('Done. Your logins are here.')
  })

  const items = ['history', 'bookmarks', 'cookies', 'passwords', 'autofill']

  it('shows the Mac heads-up and real progress counts while importing', () => {
    const result = chat(
      snapshot({
        status: 'importing',
        progress: {
          completedItems: ['history', 'bookmarks', 'cookies'],
          totalItems: 5,
        },
      }),
      requested,
    )
    expect(texts(result)).toContain('Bring my logins')
    expect(bubbles(result).map((bubble) => bubble.kind)).toContain('keychain')
    expect(bubbles(result).at(-1)).toMatchObject({
      kind: 'progress',
      text: 'Bringing your logins… 3 of 5',
      value: 0.6,
    })
    expect(chipsOf(result).editable).toBe(false)
    expect(result.dock).toEqual([])
  })

  it('skips the heads-up off a Mac and for imports without logins', () => {
    const importing = snapshot({ status: 'importing' })
    expect(
      bubbles(chat(importing, requested, 'other')).some(
        (bubble) => bubble.kind === 'keychain',
      ),
    ).toBe(false)
    const noLogins = chat(
      snapshot({ status: 'importing', sources: [WINDOWS_STYLE_SOURCE] }),
      requested,
    )
    expect(noLogins.turns.flatMap((turn) => turn.bubbles).at(-1)).toMatchObject(
      {
        text: 'Bringing them over… 0 of 3',
      },
    )
    expect(bubbles(noLogins).some((bubble) => bubble.kind === 'keychain')).toBe(
      false,
    )
  })

  it('ends on a true line built from what native reported done', () => {
    const logins = chat(
      snapshot({
        status: 'succeeded',
        progress: { completedItems: items as never, totalItems: 5 },
      }),
      requested,
    )
    expect(texts(logins).at(-1)).toBe('Done. Your logins are here.')
    const plain = chat(
      snapshot({
        status: 'succeeded',
        sources: [WINDOWS_STYLE_SOURCE],
        progress: { completedItems: ['bookmarks', 'history'], totalItems: 2 },
      }),
      requested,
    )
    expect(texts(plain).at(-1)).toBe(
      'Done. Your bookmarks and history are here.',
    )
    expect(plain.dock).toEqual([])
  })

  it('reports a failure under any status and reopens the choices', () => {
    const result = chat(
      snapshot({
        status: 'ready',
        error: {
          code: 'user_interaction_required',
          message:
            'Click Import from the visible onboarding window to continue.',
        },
      }),
      requested,
    )
    expect(texts(result)).toContain('I couldn’t bring them over.')
    expect(chipsOf(result).editable).toBe(true)
    expect(dockLabels(result).at(-1)).toBe('Try again')
  })

  it('keeps the done line after setup reports completed', () => {
    const result = chat(
      snapshot({
        status: 'completed',
        setupState: 'ready',
        progress: { completedItems: ['history'], totalItems: 1 },
        results: [
          {
            sourceId: WORK.id,
            displayName: WORK.displayName,
            status: 'succeeded',
          },
        ],
      }),
      requested,
    )
    expect(texts(result)).toContain('Done. Your history is here.')
  })
})

describe('Windows', () => {
  // The owner's rule wins over the data: Chromium lists cookies and passwords
  // on Windows but skips App-Bound values, so the page never offers them.
  const windows = (state: BrowserOSOnboardingState, choices = onLogins) =>
    chat(state, choices, 'windows')

  it('offers no login chips even when the source supports them', () => {
    const result = windows(snapshot())
    expect(texts(result)).toContain('Want your Chrome things here?')
    expect(chipsOf(result).chips.map((chip) => [chip.label, chip.on])).toEqual([
      ['Bookmarks', true],
      ['History', true],
      ['Autofill', true],
      ['Extensions', false],
      ['Search engines', false],
    ])
    expect(dockLabels(result).at(-1)).toBe('Bring them over')
  })

  it('never sends cookies or passwords, whatever was toggled', () => {
    const toggled = chatChoicesReducer(onLogins, {
      type: 'toggleItem',
      source: WORK,
      item: 'cookies',
    })
    for (const choices of [onLogins, toggled]) {
      expect(
        chatImportRequest({ state: snapshot(), choices, platform: 'windows' }),
      ).toEqual({
        sourceId: WORK.id,
        items: ['history', 'bookmarks', 'autofill'],
      })
    }
    expect(
      chatImportRequest({
        state: snapshot(),
        choices: onLogins,
        platform: 'mac',
      })?.items,
    ).toContain('cookies')
  })

  it('skips the Mac heads-up and ends on the round-4 line', () => {
    const importing = windows(
      snapshot({
        status: 'importing',
        progress: { completedItems: [], totalItems: 3 },
      }),
      requested,
    )
    expect(
      bubbles(importing).some((bubble) => bubble.kind === 'keychain'),
    ).toBe(false)
    expect(texts(importing).at(-1)).toBe('Bringing them over… 0 of 3')
    const done = windows(
      snapshot({
        status: 'succeeded',
        progress: {
          completedItems: ['history', 'bookmarks', 'autofill'],
          totalItems: 3,
        },
      }),
      requested,
    )
    expect(texts(done).at(-1)).toBe(
      'Done. Your bookmarks and history are here.',
    )
  })
})

describe('Leaving', () => {
  it('Reconnect hands straight to setup with the person’s reply', () => {
    const choices = chatChoicesReducer(initialChatChoices, {
      type: 'exit',
      reply: 'Reconnect',
    })
    const result = chat(snapshot({ setupState: 'preparing' }), choices)
    expect(result.progress).toBe(3)
    expect(texts(result).slice(-2)).toEqual([
      'Reconnect',
      'Getting your cockpit ready…',
    ])
    expect(result.dock).toEqual([])
  })

  it('Not now locks the chips and shows setup', () => {
    const choices = chatChoicesReducer(onLogins, {
      type: 'exit',
      reply: 'Not now',
    })
    const result = chat(snapshot({ setupState: 'preparing' }), choices)
    expect(chipsOf(result).editable).toBe(false)
    expect(texts(result).slice(-2)).toEqual([
      'Not now',
      'Getting your cockpit ready…',
    ])
  })

  it('offers a retry when native setup fails', () => {
    const result = chat(snapshot({ setupState: 'failed' }), {
      ...requested,
      exitReply: 'Not now',
    })
    expect(texts(result).at(-1)).toBe(
      'Setup didn’t finish. Let’s try that again.',
    )
    expect(result.dock).toEqual([
      { kind: 'primary', label: 'Try again', action: 'retrySetup' },
    ])
  })
})

describe('never uses jargon the person would have to decode', () => {
  it('keeps MCP, server, endpoint, terminal and config out of every line', () => {
    const states = [
      chat(snapshot(), initialChatChoices),
      chat(snapshot()),
      chat(snapshot({ status: 'importing' }), requested),
      chat(snapshot({ status: 'succeeded' }), requested),
      chat(snapshot({ setupState: 'failed' }), requested),
    ]
    const copy = states
      .flatMap((result) => [...texts(result), ...dockLabels(result)])
      .join(' ')
    expect(copy).not.toMatch(/\b(MCP|server|endpoint|terminal|config)\b/i)
  })
})
