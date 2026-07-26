import type { ReactNode } from 'react'

export interface DisplayHeadingProps {
  children: ReactNode
}

/** Renders the shared display heading for each onboarding step. */
export function DisplayHeading({ children }: DisplayHeadingProps) {
  return (
    <h1 className="mb-[14px] font-extrabold font-sans text-[38px] text-ink leading-[1.05] tracking-tight">
      {children}
    </h1>
  )
}

export interface EmProps {
  children: ReactNode
}

export function Em({ children }: EmProps) {
  return (
    <span className="font-medium font-serif text-accent italic">
      {children}
    </span>
  )
}

export interface StepCopyProps {
  children: ReactNode
  className?: string
}

export function StepCopy({ children, className = '' }: StepCopyProps) {
  return (
    <p
      className={`mb-[22px] max-w-[470px] text-[14.5px] text-ink-2 leading-[1.55] ${className}`}
    >
      {children}
    </p>
  )
}
