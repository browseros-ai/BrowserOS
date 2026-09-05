import type { ReactNode } from 'react'

interface StepWrapProps {
  children: ReactNode
  className?: string
}

/** Shared content width for each onboarding step. The entrance motion is
 * owned by the step-transition wrapper in Onboarding. */
export function StepWrap({
  children,
  className = 'max-w-[560px]',
}: StepWrapProps) {
  return <div className={`w-full ${className}`}>{children}</div>
}
