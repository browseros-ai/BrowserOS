import { describe, expect, it } from 'bun:test'
import { zodResolver } from '@hookform/resolvers/zod'
import { renderToStaticMarkup } from 'react-dom/server'
import { useForm } from 'react-hook-form'
import { MemoryRouter } from 'react-router'
import { Form } from '@/components/ui/form'
import {
  type OnboardingFormValues,
  onboardingFormDefaults,
  onboardingFormSchema,
} from '../onboarding-v2.schemas'
import type { ImportPhase } from '../onboarding-v2.types'
import { ImportStep } from './ImportStep'

function Harness({
  phase,
  progress = 0,
}: {
  phase: ImportPhase
  progress?: number
}) {
  const form = useForm<OnboardingFormValues>({
    resolver: zodResolver(onboardingFormSchema),
    defaultValues: onboardingFormDefaults,
  })
  return (
    <Form {...form}>
      <ImportStep
        phase={phase}
        progress={progress}
        form={form}
        onQuitChrome={() => undefined}
        onImport={() => undefined}
        onContinue={() => undefined}
      />
    </Form>
  )
}

function render(phase: ImportPhase, progress = 0): string {
  return renderToStaticMarkup(
    <MemoryRouter>
      <Harness phase={phase} progress={progress} />
    </MemoryRouter>,
  )
}

describe('ImportStep', () => {
  it('renders the Chrome-is-open notice in pre-quit phase', () => {
    const html = render('pre-quit')
    expect(html).toContain('Chrome is open')
    expect(html).toContain('Quit Chrome for me')
  })

  it('renders the picker, the Keychain notice, and an Import button in picker phase', () => {
    const html = render('picker')
    expect(html).toContain('Choose which Chrome profiles to import')
    expect(html).toContain('Work')
    expect(html).toContain('Personal')
    expect(html).toContain('Testing')
    expect(html).toContain('macOS will ask permission')
    // Default selection sums to 47 sites across 2 profiles.
    expect(html).toContain('Import 47 sites from 2 profiles')
  })

  it('renders the importing progress card during importing phase', () => {
    const html = render('importing', 12)
    expect(html).toContain('Importing sessions')
    expect(html).toContain('12 / 47 sites')
  })

  it('renders the success card and Connect-to-Claude CTA in imported phase', () => {
    const html = render('imported')
    expect(html).toContain('Imported 47 sites from 2 profiles')
    expect(html).toContain('Passwords stored in vault')
    expect(html).toContain('Connect to Claude')
  })
})
