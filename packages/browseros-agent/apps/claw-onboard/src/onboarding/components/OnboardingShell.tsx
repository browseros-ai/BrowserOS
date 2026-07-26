import type { ReactNode } from 'react'
import { StepDots } from './StepDots'
import { VisualRail } from './VisualRail'

interface OnboardingShellProps {
  step: number
  totalSteps: number
  children: ReactNode
}

/** Full-bleed onboarding frame: visual rail + scrollable step column, sized to fill the embedded popup viewport. */
export function OnboardingShell({
  step,
  totalSteps,
  children,
}: OnboardingShellProps) {
  return (
    <div className="flex h-screen overflow-hidden bg-bg-canvas">
      <VisualRail />
      <main className="flex min-h-0 flex-1 flex-col overflow-y-auto px-12 pt-11 pb-10">
        <div className="mb-[30px]">
          <StepDots step={step} total={totalSteps} />
        </div>
        {children}
      </main>
    </div>
  )
}
