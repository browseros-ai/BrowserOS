import type { ReactNode } from 'react'

interface StepWrapProps {
  children: ReactNode
}

/** Applies the shared content width and entrance animation for each step. */
export function StepWrap({ children }: StepWrapProps) {
  return <div className="w-full max-w-[560px] animate-fade-up">{children}</div>
}
