import type { ReactNode } from 'react'

interface StepWrapProps {
  children: ReactNode
  className?: string
}

/** Applies the shared content width and entrance animation for each step. */
export function StepWrap({
  children,
  className = 'max-w-[560px]',
}: StepWrapProps) {
  return <div className={`w-full animate-fade-up ${className}`}>{children}</div>
}
