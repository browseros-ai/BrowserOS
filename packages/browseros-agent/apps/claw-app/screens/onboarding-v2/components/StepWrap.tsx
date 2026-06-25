import type { ReactNode } from 'react'

interface StepWrapProps {
  children: ReactNode
}

/**
 * Width cap + fade-up animation for every step's content. Caps at
 * 560 px so the content does not stretch into the visual rail's
 * width territory.
 */
export function StepWrap({ children }: StepWrapProps) {
  return <div className="w-full max-w-[560px] animate-fade-up">{children}</div>
}
