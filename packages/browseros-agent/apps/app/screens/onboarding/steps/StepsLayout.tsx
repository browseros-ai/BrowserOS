import { ArrowLeft, Check } from 'lucide-react'
import { AnimatePresence } from 'motion/react'
import { useEffect, useState } from 'react'
import { NavLink, useNavigate, useParams } from 'react-router'
import { Button } from '@/components/ui/button'
import { ONBOARDING_STEP_VIEWED_EVENT } from '@/lib/constants/analyticsEvents'
import { track } from '@/lib/metrics/track'
import { RpcClientProvider } from '@/lib/rpc/RpcClientProvider'
import type { StepDirection } from './StepTransition'
import { steps } from './steps'

export const StepsLayout = () => {
  const { stepId } = useParams()
  const navigate = useNavigate()
  const [direction, setDirection] = useState<StepDirection>(1)

  const currentStep = Number(stepId)
  const isLastStep = currentStep >= steps.length
  const canGoPrevious = currentStep > 1

  const stepEntry = steps.find((each) => each.id === currentStep)
  const ActiveStep = stepEntry?.component ?? (() => null)

  // biome-ignore lint/correctness/useExhaustiveDependencies: track on step navigation only, stepEntry is derived from currentStep
  useEffect(() => {
    if (stepEntry) {
      track(ONBOARDING_STEP_VIEWED_EVENT, {
        step: stepEntry.id,
        step_name: stepEntry.name,
      })
    }
  }, [currentStep])

  const onContinue = () => {
    setDirection(1)
    if (isLastStep) {
      navigate('/onboarding/demo')
    } else {
      navigate(`/onboarding/steps/${currentStep + 1}`)
    }
  }

  return (
    <RpcClientProvider>
      <div className="flex h-screen flex-col overflow-hidden bg-background">
        {/* Progress Indicator */}
        <div className="border-border/40 border-b">
          <div className="mx-auto max-w-3xl px-6 py-5">
            <div className="relative flex items-center justify-between">
              {steps.map((step) => {
                const isCompleted = step.id < currentStep
                const isActive = step.id === currentStep

                return (
                  <div
                    key={step.id}
                    className="relative flex flex-1 items-center justify-center"
                  >
                    <div className="relative z-10 flex flex-col items-center gap-2">
                      <div className="relative">
                        {isActive && (
                          <div className="absolute inset-0 animate-ping rounded-full bg-[var(--accent-orange)] opacity-30" />
                        )}
                        <div
                          className={`relative flex h-8 w-8 items-center justify-center rounded-full font-semibold text-sm transition-all duration-500 ${
                            isCompleted
                              ? 'bg-[var(--accent-orange)] text-white'
                              : isActive
                                ? 'bg-[var(--accent-orange)] text-white ring-4 ring-[var(--accent-orange)]/20'
                                : 'border border-border bg-muted text-muted-foreground'
                          }`}
                        >
                          {isCompleted ? (
                            <Check className="h-4 w-4" />
                          ) : (
                            step.id
                          )}
                        </div>
                      </div>
                      <div className="hidden text-center md:block">
                        <div
                          className={`font-medium text-xs transition-colors duration-300 ${
                            isCompleted || isActive
                              ? 'text-foreground'
                              : 'text-muted-foreground'
                          }`}
                        >
                          {step.name}
                        </div>
                      </div>
                    </div>
                  </div>
                )
              })}
            </div>
          </div>
        </div>

        {/* Main Content */}
        <main className="flex flex-1 items-center justify-center overflow-y-auto overflow-x-hidden px-6">
          <div className="w-full max-w-4xl">
            <div className="relative h-[550px]">
              <AnimatePresence initial={false} custom={direction}>
                <ActiveStep
                  key={currentStep}
                  direction={direction}
                  onContinue={onContinue}
                />
              </AnimatePresence>
            </div>
            <div className="pt-8">
              <Button variant="ghost" asChild className="group">
                <NavLink
                  onClick={() => setDirection(-1)}
                  to={
                    canGoPrevious
                      ? `/onboarding/steps/${currentStep - 1}`
                      : '/onboarding'
                  }
                >
                  <ArrowLeft className="mr-2 h-4 w-4 transition-transform group-hover:-translate-x-0.5" />
                  Back
                </NavLink>
              </Button>
            </div>
          </div>
        </main>
      </div>
    </RpcClientProvider>
  )
}
