/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { useEffect, useRef, useState } from 'react'
import { useForm } from 'react-hook-form'
import { Form } from '@/components/ui/form'
import {
  BROWSEROS_ONBOARDING_API_VERSION,
  type BrowserOSImportStatus,
  type BrowserOSOnboardingState,
} from './browseros-onboarding-api'
import { createBrowserOSOnboardingBridge } from './browseros-onboarding-bridge'
import { OnboardingShell } from './components/OnboardingShell'
import {
  importSourceSelectionChangeFor,
  selectedSourceById,
  startImportRequestFor,
} from './onboarding-v2.helpers'
import {
  type OnboardingFormValues,
  onboardingFormDefaults,
  onboardingFormResolver,
} from './onboarding-v2.schemas'
import type { ImportPhase, Step } from './onboarding-v2.types'
import { ImportStep } from './steps/ImportStep'
import { ReadyStep } from './steps/ReadyStep'
import { WelcomeStep } from './steps/WelcomeStep'

const TOTAL_STEPS = 3
const BROWSEROS_NEW_TAB_URL = 'chrome://newtab'

const initialOnboardingState: BrowserOSOnboardingState = {
  apiVersion: BROWSEROS_ONBOARDING_API_VERSION,
  status: 'idle',
  sources: [],
}

/** Maps Chromium importer status into the local three-step onboarding screen state. */
export function importPhaseFor(
  status: BrowserOSImportStatus,
  hasPreparedForImport: boolean,
): ImportPhase {
  if (status === 'importing') return 'importing'
  if (status === 'failed') return 'failed'
  if (status === 'succeeded') return 'imported'
  if (!hasPreparedForImport) return 'pre-quit'
  return 'picker'
}

/** Leaves standalone onboarding for BrowserOS's Chromium new-tab page. */
export function openBrowserOsNewTab() {
  window.location.assign(BROWSEROS_NEW_TAB_URL)
}

/** Runs the standalone three-step BrowserClaw onboarding flow. */
export function OnboardingV2() {
  const form = useForm<OnboardingFormValues>({
    resolver: onboardingFormResolver,
    defaultValues: onboardingFormDefaults,
    mode: 'onChange',
  })

  const [step, setStep] = useState<Step>(0)
  const [bridge] = useState(() => createBrowserOSOnboardingBridge())
  const [onboardingState, setOnboardingState] =
    useState<BrowserOSOnboardingState>(initialOnboardingState)
  const [hasPreparedForImport, setHasPreparedForImport] = useState(false)
  const didNotifyPageReady = useRef(false)
  const importPhase = importPhaseFor(
    onboardingState.status,
    hasPreparedForImport,
  )

  useEffect(() => {
    const cleanup = bridge.registerReceiver(setOnboardingState)
    if (!didNotifyPageReady.current) {
      didNotifyPageReady.current = true
      bridge.pageReady()
    }
    return cleanup
  }, [bridge])

  useEffect(() => {
    const currentSourceId = form.getValues('selectedSourceId')
    const selectionChange = importSourceSelectionChangeFor(
      onboardingState.sources,
      currentSourceId,
    )
    if (!selectionChange) return
    if (selectionChange.selectedSourceId !== currentSourceId) {
      form.setValue('selectedSourceId', selectionChange.selectedSourceId, {
        shouldValidate: true,
      })
    }
    if (selectionChange.selectedItems.length === 0) {
      if (form.getValues('selectedItems').length > 0) {
        form.setValue('selectedItems', [], { shouldValidate: true })
      }
      return
    }
    form.setValue('selectedItems', selectionChange.selectedItems, {
      shouldValidate: true,
    })
  }, [form, onboardingState.sources])

  function prepareForImport() {
    setHasPreparedForImport(true)
    bridge.refreshSources()
  }

  function startImport() {
    const source = selectedSourceById(
      onboardingState.sources,
      form.getValues('selectedSourceId'),
    )
    if (!source) return
    const request = startImportRequestFor(
      source,
      form.getValues('selectedItems'),
    )
    if (!request) return
    bridge.startImport(request)
  }

  function finishOnboarding() {
    bridge.complete()
    openBrowserOsNewTab()
  }

  return (
    <Form {...form}>
      <OnboardingShell step={step} totalSteps={TOTAL_STEPS}>
        {step === 0 && (
          <WelcomeStep onPrimary={() => setStep(1)} onSkip={() => setStep(2)} />
        )}
        {step === 1 && (
          <ImportStep
            phase={importPhase}
            state={onboardingState}
            form={form}
            onQuitChrome={prepareForImport}
            onImport={startImport}
            onRefresh={() => bridge.refreshSources()}
            onContinue={() => setStep(2)}
          />
        )}
        {step === 2 && <ReadyStep onDone={finishOnboarding} />}
      </OnboardingShell>
    </Form>
  )
}
