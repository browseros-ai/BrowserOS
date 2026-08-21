/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { motion, useReducedMotion } from 'motion/react'
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
import { SetupAgentStep } from './steps/SetupAgentStep'
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

/** Runs the standalone three-step BrowserOS onboarding flow. */
export function Onboarding() {
  const reduce = useReducedMotion()
  const form = useForm<OnboardingFormValues>({
    resolver: onboardingFormResolver,
    defaultValues: onboardingFormDefaults,
    mode: 'onChange',
  })

  const [step, setStep] = useState<Step>(0)
  const direction = useRef(1)
  const [bridge] = useState(() => createBrowserOSOnboardingBridge())
  const [onboardingState, setOnboardingState] =
    useState<BrowserOSOnboardingState>(initialOnboardingState)
  const didNotifyPageReady = useRef(false)
  const importPhase = importPhaseFor(onboardingState.status)

  function goTo(next: Step) {
    direction.current = next >= step ? 1 : -1
    setStep(next)
  }

  useEffect(() => {
    const cleanup = bridge.registerReceiver(setOnboardingState)
    if (!didNotifyPageReady.current) {
      didNotifyPageReady.current = true
      bridge.pageReady()
    }
    return cleanup
  }, [bridge])

  // Sync the picker form to whatever profiles the native side reports. This is a
  // subscription to an external source (the chrome.send bridge), so it stays an
  // effect rather than derived render state.
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

  /**
   * Signals the native first-run to finish. The landing (the app's #/settings/ai
   * pane) is injected natively as a first-run tab; the SPA only reports done, so
   * gating this on the bridge (as an earlier BrowserOS neo build did) is what
   * left the button dead in the shipped browser.
   */
  function finishOnboarding() {
    bridge.complete()
  }

  return (
    <Form {...form}>
      <OnboardingShell step={step} totalSteps={TOTAL_STEPS}>
        {/* Re-mounts on each step (keyed) and plays a directional slide-in.
            No exit animation, so the flow can never stall waiting on one. */}
        <motion.div
          key={step}
          initial={
            reduce
              ? false
              : { x: direction.current >= 0 ? 40 : -40, opacity: 0 }
          }
          animate={{ x: 0, opacity: 1 }}
          transition={{
            type: 'spring',
            stiffness: 300,
            damping: 30,
            opacity: { duration: 0.2 },
          }}
        >
          {step === 0 && (
            <WelcomeStep onPrimary={() => goTo(1)} onSkip={finishOnboarding} />
          )}
          {step === 1 && (
            <ImportStep
              phase={importPhase}
              state={onboardingState}
              form={form}
              onImport={startImport}
              onRefresh={() => bridge.refreshSources()}
              onContinue={() => goTo(2)}
            />
          )}
          {step === 2 && (
            <SetupAgentStep
              onSetup={finishOnboarding}
              onLater={finishOnboarding}
            />
          )}
        </motion.div>
      </OnboardingShell>
    </Form>
  )
}
