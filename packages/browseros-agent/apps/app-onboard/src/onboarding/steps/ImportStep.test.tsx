import { describe, expect, it } from 'bun:test'
import { renderToStaticMarkup } from 'react-dom/server'
import { useForm } from 'react-hook-form'
import { MemoryRouter } from 'react-router'
import { Form } from '@/components/ui/form'
import type { BrowserOSOnboardingState } from '../browseros-onboarding-api'
import { BROWSEROS_ONBOARDING_API_VERSION } from '../browseros-onboarding-api'
import {
  defaultImportItemsForSource,
  importItemLabel,
  MOCK_BROWSEROS_IMPORT_SOURCES,
} from '../onboarding-v2.helpers'
import {
  type OnboardingFormValues,
  onboardingFormDefaults,
  onboardingFormResolver,
  onboardingFormSchema,
} from '../onboarding-v2.schemas'
import type { ImportPhase } from '../onboarding-v2.types'
import { ImportStep } from './ImportStep'

// Take the message from the resolver rather than restating it, so these tests
// track the production copy instead of a string they hardcoded themselves.
const unpickedResult = onboardingFormSchema.safeParse({
  selectedSourceId: '',
  selectedItems: [],
})
const PICK_PROFILE_MESSAGE = unpickedResult.success
  ? ''
  : (unpickedResult.error.issues[0]?.message ?? '')

function readyState(
  overrides: Partial<BrowserOSOnboardingState> = {},
): BrowserOSOnboardingState {
  return {
    apiVersion: BROWSEROS_ONBOARDING_API_VERSION,
    status: 'ready',
    sources: [...MOCK_BROWSEROS_IMPORT_SOURCES],
    ...overrides,
  }
}

function Harness({
  phase,
  state = readyState(),
  formValues = {},
  unpicked = false,
}: {
  phase: ImportPhase
  state?: BrowserOSOnboardingState
  formValues?: Partial<OnboardingFormValues>
  unpicked?: boolean
}) {
  const form = useForm<OnboardingFormValues>({
    resolver: onboardingFormResolver,
    defaultValues: { ...onboardingFormDefaults, ...formValues },
  })
  // Reproduces the error OnboardingV2 sets on itself: its sources effect calls
  // setValue('selectedSourceId', '', { shouldValidate: true }), and the resolver
  // rejects the empty id. Seeding it here is the only way to get a form error
  // into a server render, nothing else validates (no events, no effects).
  if (unpicked) {
    form.setError('selectedSourceId', {
      type: 'required',
      message: PICK_PROFILE_MESSAGE,
    })
  }
  return (
    <Form {...form}>
      <ImportStep
        phase={phase}
        state={state}
        form={form}
        onImport={() => undefined}
        onRefresh={() => undefined}
        onContinue={() => undefined}
      />
    </Form>
  )
}

function render(
  phase: ImportPhase,
  state: BrowserOSOnboardingState = readyState(),
  formValues: Partial<OnboardingFormValues> = {},
  unpicked = false,
): string {
  return renderToStaticMarkup(
    <MemoryRouter>
      <Harness
        phase={phase}
        state={state}
        formValues={formValues}
        unpicked={unpicked}
      />
    </MemoryRouter>,
  )
}

function checklistRowFor(html: string, label: string): string {
  const checklistStart = html.indexOf('What to copy')
  const labelIndex = html.indexOf(`>${label}</span>`, checklistStart)
  if (checklistStart === -1 || labelIndex === -1) return ''
  const rowStart = html.lastIndexOf('<label', labelIndex)
  const rowEnd = html.indexOf('</label>', labelIndex)
  return html.slice(rowStart, rowEnd + '</label>'.length)
}

// The empty picker renders a disabled import CTA next to an enabled skip, so
// `disabled` assertions have to be scoped to one button's own markup. Labels can
// also appear outside buttons ("Pick a profile" is a prefix of the "Pick a
// profile to import" heading), so skip matches that aren't inside a button.
function buttonMarkupFor(html: string, label: string): string {
  for (
    let labelIndex = html.indexOf(label);
    labelIndex !== -1;
    labelIndex = html.indexOf(label, labelIndex + 1)
  ) {
    const buttonStart = html.lastIndexOf('<button', labelIndex)
    const buttonEnd = html.indexOf('</button>', labelIndex)
    if (buttonStart === -1 || buttonEnd === -1) continue
    if (html.indexOf('</button>', buttonStart) !== buttonEnd) continue
    return html.slice(buttonStart, buttonEnd + '</button>'.length)
  }
  return ''
}

describe('ImportStep', () => {
  it('renders a compact picker without the permission dialog before Copy', () => {
    const html = render('picker')
    expect(html).toContain('Pick a profile')
    expect(html).toContain('Work')
    expect(html).toContain('Personal')
    expect(html).toContain('Microsoft Edge')
    expect(html).toContain('What to copy')
    for (const item of defaultImportItemsForSource(
      MOCK_BROWSEROS_IMPORT_SOURCES[0],
    )) {
      expect(checklistRowFor(html, importItemLabel(item))).toContain(
        'aria-checked="true"',
      )
    }
    expect(html).toContain('5 categories selected')
    expect(html).not.toContain('Example macOS dialog')
    expect(html).not.toContain('Always Allow')
    expect(html).not.toContain('/icon/keychain-prompt.png')
    expect(html).toContain('Copy from Chrome')
    expect(html).not.toContain('Chrome is open')
    expect(html).not.toContain('Quit Chrome for me')
    expect(html).not.toContain('disabled=""')
  })

  it('checks every supported non-optional item by default', () => {
    const source = MOCK_BROWSEROS_IMPORT_SOURCES[1]
    const html = render('picker', readyState(), {
      selectedSourceId: source.id,
      selectedItems: defaultImportItemsForSource(source),
    })

    expect(html).toContain('5 categories selected')
    expect(html).toContain('Copy from Chrome')
    for (const item of [
      'History',
      'Bookmarks',
      'Signed-in sessions (cookies)',
      'Saved passwords',
      'Autofill',
    ]) {
      const row = checklistRowFor(html, item)
      expect(row).toContain('data-checked=""')
      expect(row).toContain('aria-checked="true"')
    }
    expect(html).not.toContain('<details')
  })

  it('collapses only search engines and extensions behind a disclosure', () => {
    const html = render('picker')
    const detailsStart = html.indexOf('<details')
    const detailsEnd = html.indexOf('</details>', detailsStart)
    const details = html.slice(detailsStart, detailsEnd)

    expect(detailsStart).toBeGreaterThan(-1)
    expect(html).toContain('Also copy my browsing setup')
    expect(details).toContain('Search engines')
    expect(details).toContain('Extensions')
    for (const item of [
      'History',
      'Bookmarks',
      'Signed-in sessions (cookies)',
      'Saved passwords',
      'Autofill',
    ]) {
      expect(details).not.toContain(item)
    }
    for (const item of ['Search engines', 'Extensions']) {
      expect(checklistRowFor(html, item)).toContain('aria-checked="false"')
    }
  })

  it('counts only the selected categories', () => {
    const html = render('picker', readyState(), {
      selectedItems: ['cookies', 'passwords', 'history'],
    })

    expect(html).toContain('3 categories selected')
    expect(html).toContain('Copy from Chrome')
  })

  it('disables the copy action until at least one item is selected', () => {
    const html = render('picker', readyState(), { selectedItems: [] })

    expect(html).toContain('0 categories selected')
    expect(html).toContain('Select what to copy')
    expect(html).toContain('disabled=""')
  })

  it('disables import while Chromium is detecting sources', () => {
    const html = render('picker', readyState({ status: 'detecting' }))
    expect(html).toContain('Looking for profiles')
    expect(html).toContain('disabled=""')
  })

  it('disables import when the selected source has no supported items', () => {
    const html = render(
      'picker',
      readyState({
        sources: [
          {
            ...MOCK_BROWSEROS_IMPORT_SOURCES[0],
            recommendedItems: [],
            supportedItems: [],
          },
        ],
      }),
    )
    expect(html).toContain('Nothing to copy from this profile')
    expect(html).not.toContain('What to copy')
    expect(html).toContain('disabled=""')
  })

  it('allows copying one selected category', () => {
    const html = render('picker', readyState(), {
      selectedItems: ['history'],
    })

    expect(html).toContain('1 categories selected')
    expect(html).toContain('Copy from Chrome')
  })

  it('renders the importing progress card during importing phase', () => {
    const html = render(
      'importing',
      readyState({
        status: 'importing',
        progress: {
          currentItem: 'cookies',
          currentSourceId: MOCK_BROWSEROS_IMPORT_SOURCES[0].id,
          currentSourceName: MOCK_BROWSEROS_IMPORT_SOURCES[0].displayName,
          completedItems: ['history', 'bookmarks'],
          totalItems: 7,
          completedSources: 0,
          totalSources: 1,
        },
        results: [
          {
            sourceId: MOCK_BROWSEROS_IMPORT_SOURCES[0].id,
            displayName: MOCK_BROWSEROS_IMPORT_SOURCES[0].displayName,
            status: 'importing',
          },
        ],
      }),
    )
    expect(html).toContain('Importing Signed-in sessions (cookies)')
    expect(html).toContain('Google Chrome - Work')
    expect(html).toContain('2 / 7 items')
  })

  it('renders a failure recovery state when Chromium reports failed', () => {
    const html = render(
      'failed',
      readyState({
        status: 'failed',
        error: {
          code: 'import_failed',
          message: 'Chrome needs to be closed before importing.',
        },
      }),
    )

    expect(html).toContain('Something went wrong.')
    expect(html).toContain('Chrome needs to be closed before importing.')
    expect(html).toContain('Try again')
    expect(html).not.toContain('Pick a profile to import')
  })

  it('renders the success card and continue CTA in imported phase', () => {
    const html = render(
      'imported',
      readyState({
        status: 'succeeded',
        progress: {
          completedItems: MOCK_BROWSEROS_IMPORT_SOURCES[0].recommendedItems,
          totalItems: 7,
          completedSources: 1,
          totalSources: 1,
        },
        results: [
          {
            sourceId: MOCK_BROWSEROS_IMPORT_SOURCES[0].id,
            displayName: MOCK_BROWSEROS_IMPORT_SOURCES[0].displayName,
            status: 'succeeded',
          },
        ],
      }),
    )
    expect(html).toContain('Copied 7 items from Google Chrome - Work')
    expect(html).toContain('History, Bookmarks')
    expect(html).toContain('Continue')
  })

  it('does not fabricate a success summary when progress is missing', () => {
    const html = render('imported', readyState({ status: 'succeeded' }))

    expect(html).toContain('Copied 0 items from Work')
    expect(html).toContain('No item details reported')
    expect(html).not.toContain('History, Bookmarks')
  })

  it('does not fall back to selected items when no completed items are reported', () => {
    const html = render(
      'imported',
      readyState({
        status: 'succeeded',
        progress: {
          completedItems: [],
          totalItems: 7,
        },
      }),
    )

    expect(html).toContain('Copied 0 items from Work')
    expect(html).toContain('No completed items reported')
    expect(html).not.toContain('History, Bookmarks')
  })

  it('leaves an enabled way forward when no profiles are found', () => {
    const html = render('picker', readyState({ sources: [] }))
    const skip = buttonMarkupFor(html, 'Skip for now')

    expect(html).toContain('No profiles found.')
    expect(buttonMarkupFor(html, 'Pick a profile')).toContain('disabled=""')
    expect(skip).toContain('Skip for now')
    expect(skip).not.toContain('disabled=""')
  })

  it('keeps the skip available when profiles are present', () => {
    const skip = buttonMarkupFor(render('picker'), 'Skip for now')

    expect(skip).toContain('Skip for now')
    expect(skip).not.toContain('disabled=""')
  })

  it('keeps the skip enabled while profiles are still being detected', () => {
    const html = render('picker', readyState({ status: 'detecting' }))
    const skip = buttonMarkupFor(html, 'Skip for now')

    expect(html).toContain('Looking for profiles')
    expect(skip).toContain('Skip for now')
    expect(skip).not.toContain('disabled=""')
  })

  it('offers a skip out of a failed import', () => {
    const html = render('failed', readyState({ status: 'failed', sources: [] }))
    const skip = buttonMarkupFor(html, 'Skip for now')

    expect(html).toContain('Try again')
    expect(html).toContain('Refresh profiles')
    expect(skip).toContain('Skip for now')
    expect(skip).not.toContain('disabled=""')
  })

  it('does not offer a skip while an import is running', () => {
    const html = render('importing', readyState({ status: 'importing' }))

    expect(html).not.toContain('Skip for now')
  })

  it('does not offer a skip once the import has succeeded', () => {
    const html = render('imported', readyState({ status: 'succeeded' }))

    expect(html).toContain('Continue')
    expect(html).not.toContain('Skip for now')
  })

  it('does not blame the user when there is no profile to pick', () => {
    const html = render('picker', readyState({ sources: [] }), {}, true)

    expect(html).toContain('No profiles found.')
    expect(html).not.toContain('data-slot="form-message"')
  })

  it('does not blame the user while profiles are still being detected', () => {
    const html = render(
      'picker',
      readyState({ status: 'detecting', sources: [] }),
      {},
      true,
    )

    expect(html).toContain('Looking for profiles')
    expect(html).not.toContain('data-slot="form-message"')
  })

  it('still asks for a pick when profiles are there to pick from', () => {
    const html = render('picker', readyState(), {}, true)

    expect(PICK_PROFILE_MESSAGE).not.toBe('')
    expect(html).toContain('data-slot="form-message"')
    expect(html).toContain(PICK_PROFILE_MESSAGE)
  })
  it('shows four profiles while keeping a selection outside the first four visible', () => {
    const sources = Array.from({ length: 7 }, (_, index) => ({
      ...MOCK_BROWSEROS_IMPORT_SOURCES[0],
      id: `profile-${index}`,
      profileName: `Person ${index}`,
    }))
    const html = render('picker', readyState({ sources }), {
      selectedSourceId: 'profile-6',
    })
    expect(html).toContain('Person 6')
    expect(html).not.toContain('Person 3')
    expect(html).not.toContain('Person 4')
    expect(html).toContain('Show 3 more profiles')
    expect(html.match(/type="radio"/g)).toHaveLength(4)
  })
})
