import { describe, expect, it } from 'bun:test'
import { renderToStaticMarkup } from 'react-dom/server'
import { useForm } from 'react-hook-form'
import { MemoryRouter } from 'react-router'
import { Form } from '@/components/ui/form'
import type { BrowserOSOnboardingState } from '../browseros-onboarding-api'
import { BROWSEROS_ONBOARDING_API_VERSION } from '../browseros-onboarding-api'
import {
  importItemLabel,
  MOCK_BROWSEROS_IMPORT_SOURCES,
  selectableItemsForSource,
} from '../onboarding-v2.helpers'
import {
  type OnboardingFormValues,
  onboardingFormDefaults,
  onboardingFormResolver,
} from '../onboarding-v2.schemas'
import type { ImportPhase } from '../onboarding-v2.types'
import { ImportStep } from './ImportStep'

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
}: {
  phase: ImportPhase
  state?: BrowserOSOnboardingState
  formValues?: Partial<OnboardingFormValues>
}) {
  const form = useForm<OnboardingFormValues>({
    resolver: onboardingFormResolver,
    defaultValues: { ...onboardingFormDefaults, ...formValues },
  })
  return (
    <Form {...form}>
      <ImportStep
        phase={phase}
        state={state}
        form={form}
        onQuitChrome={() => undefined}
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
): string {
  return renderToStaticMarkup(
    <MemoryRouter>
      <Harness phase={phase} state={state} formValues={formValues} />
    </MemoryRouter>,
  )
}

function checklistRowFor(html: string, label: string): string {
  const checklistStart = html.indexOf('What to import')
  const labelIndex = html.indexOf(`>${label}</span>`, checklistStart)
  if (checklistStart === -1 || labelIndex === -1) return ''
  const rowStart = html.lastIndexOf('<label', labelIndex)
  const rowEnd = html.indexOf('</label>', labelIndex)
  return html.slice(rowStart, rowEnd + '</label>'.length)
}

describe('ImportStep', () => {
  it('renders the Chrome-is-open notice in pre-quit phase', () => {
    const html = render('pre-quit')
    expect(html).toContain('Chrome is open')
    expect(html).toContain('Quit Chrome for me')
  })

  it('renders the picker, the Keychain notice, and an Import button in picker phase', () => {
    const html = render('picker')
    expect(html).toContain('Choose a browser profile to import')
    expect(html).toContain('Google Chrome - Work')
    expect(html).toContain('Google Chrome - Personal')
    expect(html).toContain('Microsoft Edge - Default')
    expect(html).toContain('What to import')
    for (const item of MOCK_BROWSEROS_IMPORT_SOURCES[0].supportedItems) {
      expect(checklistRowFor(html, importItemLabel(item))).toContain(
        'aria-checked="true"',
      )
    }
    expect(html).toContain('7 of 7 selected')
    expect(html).toContain('macOS will ask permission')
    expect(html).toContain('Import 7 items from Work')
    expect(html).not.toContain('disabled=""')
  })

  it('checks recommended items and leaves non-recommended supported items unchecked', () => {
    const source = MOCK_BROWSEROS_IMPORT_SOURCES[1]
    const html = render('picker', readyState(), {
      selectedSourceId: source.id,
      selectedItems: selectableItemsForSource(source),
    })

    expect(html).toContain('4 of 5 selected')
    expect(html).toContain('Import 4 items from Personal')
    for (const item of source.recommendedItems) {
      const row = checklistRowFor(html, importItemLabel(item))
      expect(row).toContain('data-checked=""')
      expect(row).toContain('aria-checked="true"')
    }
    const autofillRow = checklistRowFor(html, 'Autofill')
    expect(autofillRow).toContain('data-unchecked=""')
    expect(autofillRow).toContain('aria-checked="false"')
  })

  it('disables import until at least one supported item is selected', () => {
    const html = render('picker', readyState(), { selectedItems: [] })

    expect(html).toContain('0 of 7 selected')
    expect(html).toContain('Select items to import')
    expect(html).toContain('disabled=""')
  })

  it('disables import while Chromium is detecting sources', () => {
    const html = render('picker', readyState({ status: 'detecting' }))
    expect(html).toContain('Detecting import sources')
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
    expect(html).toContain('No supported import items')
    expect(html).not.toContain('What to import')
    expect(html).toContain('disabled=""')
  })

  it('uses the singular source tile item count for one supported item', () => {
    const html = render(
      'picker',
      readyState({
        sources: [
          {
            ...MOCK_BROWSEROS_IMPORT_SOURCES[0],
            supportedItems: ['history'],
            recommendedItems: ['history'],
          },
        ],
      }),
    )

    expect(html).toContain('1 item')
    expect(html).not.toContain('1 items')
  })

  it('uses the singular item label when one item is selected', () => {
    const html = render('picker', readyState(), {
      selectedItems: ['history'],
    })

    expect(html).toContain('1 of 7 selected')
    expect(html).toContain('Import 1 item from Work')
  })

  it('renders the importing progress card during importing phase', () => {
    const html = render(
      'importing',
      readyState({
        status: 'importing',
        progress: {
          currentItem: 'cookies',
          completedItems: ['history', 'bookmarks'],
          totalItems: 7,
        },
      }),
    )
    expect(html).toContain('Importing Cookies')
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

    expect(html).toContain('Import failed')
    expect(html).toContain('Chrome needs to be closed before importing.')
    expect(html).toContain('Try import again')
    expect(html).not.toContain('Choose a browser profile to import')
  })

  it('renders the success card and continue CTA in imported phase', () => {
    const html = render(
      'imported',
      readyState({
        status: 'succeeded',
        progress: {
          completedItems: MOCK_BROWSEROS_IMPORT_SOURCES[0].recommendedItems,
          totalItems: 7,
        },
      }),
    )
    expect(html).toContain('Imported 7 items from Work')
    expect(html).toContain('History, Bookmarks')
    expect(html).toContain('Continue')
  })

  it('does not fabricate a success summary when progress is missing', () => {
    const html = render('imported', readyState({ status: 'succeeded' }))

    expect(html).toContain('Imported 0 items from Work')
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

    expect(html).toContain('Imported 0 items from Work')
    expect(html).toContain('No completed items reported')
    expect(html).not.toContain('History, Bookmarks')
  })
})
