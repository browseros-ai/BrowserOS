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
import { SetupStep } from './steps/SetupStep'
import { WelcomeStep } from './steps/WelcomeStep'

const TOTAL_STEPS = 3

const initialOnboardingState: BrowserOSOnboardingState = {
  apiVersion: BROWSEROS_ONBOARDING_API_VERSION,
  status: 'idle',
  sources: [],
}

/** Maps Chromium importer status into the local three-step onboarding screen state. */
export function importPhaseFor(status: BrowserOSImportStatus): ImportPhase {
  if (status === 'importing') return 'importing'
  if (status === 'failed') return 'failed'
  if (status === 'succeeded') return 'imported'
  return 'picker'
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
  const didNotifyPageReady = useRef(false)
  const importPhase = importPhaseFor(onboardingState.status)
  const isFinishing = Boolean(
    onboardingState.setupState && onboardingState.setupState !== 'idle',
  )

  useEffect(() => {
    // Install first: pageReady may synchronously restore an in-flight setup
    // after reload. Receiving it only renders state; it never sends COMPLETE.
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

  // The bridge publishes preparing before sending the finish request and
  // suppresses repeated exits. Chromium alone decides when it is safe to leave.
  function finishOnboarding() {
    bridge.complete()
  }

  return (
    <Form {...form}>
      <OnboardingShell
        step={step}
        totalSteps={TOTAL_STEPS}
        showProgress={!isFinishing}
      >
        {isFinishing ? (
          <SetupStep
            failed={onboardingState.setupState === 'failed'}
            onRetry={() => bridge.retrySetup()}
          />
        ) : (
          <>
            {step === 0 && (
              <WelcomeStep
                onPrimary={() => setStep(1)}
                onSkip={finishOnboarding}
              />
            )}
            {step === 1 && (
              <ImportStep
                phase={importPhase}
                state={onboardingState}
                form={form}
                onImport={startImport}
                onRefresh={() => bridge.refreshSources()}
                onContinue={() => setStep(2)}
              />
            )}
            {step === 2 && <ReadyStep onDone={finishOnboarding} />}
          </>
        )}
      </OnboardingShell>
    </Form>
  )
}
