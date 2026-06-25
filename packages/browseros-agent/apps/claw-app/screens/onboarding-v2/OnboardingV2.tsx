/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * v2 onboarding shell. Four steps inside one macwin frame with a
 * persistent visual rail. UI-only: every "action" advances local
 * state or runs a fake-progress timer. The wiring person hooks real
 * mutations into the four callbacks below without touching layout,
 * typography, or the form-state model.
 *
 * Wiring-person checklist:
 *
 *   - `onQuitChrome`     : replace with a real "quit Chrome safely" call.
 *   - `onImport`         : replace the fake-progress effect below with a
 *                          subscription to the import service that drives
 *                          `setImportProgress` for real.
 *   - `onAddToClaude`    : replace the connect timer with a mutation
 *                          against the connect endpoint; flip
 *                          `setConnectPhase` on the mutation's lifecycle.
 *   - `onDone`           : confirm the navigation target is right for the
 *                          post-onboarding home (cockpit homepage today).
 *
 * `form.getValues().selectedProfileIds` is the picker's output. RHF
 * keeps it validated, typed, and stable across step navigations.
 */

import { zodResolver } from '@hookform/resolvers/zod'
import { useEffect, useState } from 'react'
import { useForm } from 'react-hook-form'
import { useNavigate } from 'react-router'
import { Form } from '@/components/ui/form'
import { OnboardingShell } from './components/OnboardingShell'
import { sumSitesFor } from './onboarding-v2.helpers'
import {
  type OnboardingFormValues,
  onboardingFormDefaults,
  onboardingFormSchema,
} from './onboarding-v2.schemas'
import type { ConnectPhase, ImportPhase, Step } from './onboarding-v2.types'
import { ConnectStep } from './steps/ConnectStep'
import { ImportStep } from './steps/ImportStep'
import { ReadyStep } from './steps/ReadyStep'
import { WelcomeStep } from './steps/WelcomeStep'

const TOTAL_STEPS = 4
const FAKE_IMPORT_TICK_MS = 70
const FAKE_IMPORT_SETTLE_MS = 350
const FAKE_CONNECT_DELAY_MS = 1700

export function OnboardingV2() {
  const navigate = useNavigate()
  const form = useForm<OnboardingFormValues>({
    resolver: zodResolver(onboardingFormSchema),
    defaultValues: onboardingFormDefaults,
    mode: 'onChange',
  })

  const [step, setStep] = useState<Step>(0)
  const [importPhase, setImportPhase] = useState<ImportPhase>('pre-quit')
  const [connectPhase, setConnectPhase] = useState<ConnectPhase>('idle')
  const [importProgress, setImportProgress] = useState(0)

  // Fake import-progress climber. The wiring person replaces this
  // entire effect with a subscription to the real import service
  // that drives `setImportProgress`.
  useEffect(() => {
    if (importPhase !== 'importing') return
    const totalSites = sumSitesFor(form.getValues().selectedProfileIds)
    if (totalSites === 0) {
      setImportPhase('imported')
      return
    }
    setImportProgress(0)
    let cursor = 0
    const tick = window.setInterval(() => {
      cursor += Math.ceil(Math.random() * 4)
      if (cursor >= totalSites) {
        cursor = totalSites
        window.clearInterval(tick)
        window.setTimeout(
          () => setImportPhase('imported'),
          FAKE_IMPORT_SETTLE_MS,
        )
      }
      setImportProgress(cursor)
    }, FAKE_IMPORT_TICK_MS)
    return () => window.clearInterval(tick)
  }, [importPhase, form])

  // Fake connect timer. The wiring person replaces this with a real
  // mutation against the connect endpoint; flip `connectPhase` on
  // the mutation's success / error lifecycle.
  useEffect(() => {
    if (connectPhase !== 'connecting') return
    const timer = window.setTimeout(
      () => setConnectPhase('connected'),
      FAKE_CONNECT_DELAY_MS,
    )
    return () => window.clearTimeout(timer)
  }, [connectPhase])

  return (
    <Form {...form}>
      <OnboardingShell step={step} totalSteps={TOTAL_STEPS}>
        {step === 0 && (
          <WelcomeStep onPrimary={() => setStep(1)} onSkip={() => setStep(3)} />
        )}
        {step === 1 && (
          <ImportStep
            phase={importPhase}
            progress={importProgress}
            form={form}
            onQuitChrome={() => setImportPhase('picker')}
            onImport={() => setImportPhase('importing')}
            onContinue={() => setStep(2)}
          />
        )}
        {step === 2 && (
          <ConnectStep
            phase={connectPhase}
            onAddToClaude={() => setConnectPhase('connecting')}
            onContinue={() => setStep(3)}
          />
        )}
        {step === 3 && <ReadyStep onDone={() => navigate('/')} />}
      </OnboardingShell>
    </Form>
  )
}
