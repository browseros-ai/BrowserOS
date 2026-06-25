import type { ReactNode } from 'react'

interface DisplayHeadingProps {
  children: ReactNode
}

/**
 * Big sans-serif heading used at the top of every onboarding step.
 * Wrap an `<Em>` around accent words ("drive", "logins", "Claude",
 * "set") to pick up the Newsreader italic serif accent.
 */
export function DisplayHeading({ children }: DisplayHeadingProps) {
  return (
    <h1 className="mb-[14px] font-extrabold font-sans text-[38px] text-ink leading-[1.05] tracking-tight">
      {children}
    </h1>
  )
}

interface EmProps {
  children: ReactNode
}

export function Em({ children }: EmProps) {
  return (
    <span className="font-medium font-serif text-accent italic">
      {children}
    </span>
  )
}

interface StepCopyProps {
  children: ReactNode
  className?: string
}

/**
 * Body copy under the display heading. Caps at 470 px so lines do
 * not stretch all the way to the content column's right edge.
 */
export function StepCopy({ children, className = '' }: StepCopyProps) {
  return (
    <p
      className={`mb-[22px] max-w-[470px] text-[14.5px] text-ink-2 leading-[1.55] ${className}`}
    >
      {children}
    </p>
  )
}
