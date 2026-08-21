import type { ReactNode } from 'react'

interface StepWrapProps {
  children: ReactNode
}

/** Shared content width for each onboarding step. The entrance motion is
 * owned by the step-transition wrapper in Onboarding. */
export function StepWrap({ children }: StepWrapProps) {
  return <div className="w-full max-w-[560px]">{children}</div>
}
