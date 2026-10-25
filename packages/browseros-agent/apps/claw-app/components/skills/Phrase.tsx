import type { ReactNode } from 'react'
import { cn } from '@/lib/utils'

/** A phrase a person types into their AI agent, such as `/neo-inbox-sweep`. */
export function Phrase({
  children,
  className,
}: {
  children: ReactNode
  className?: string
}) {
  return (
    <span
      className={cn(
        'rounded-full bg-accent-tint px-2 py-0.5 font-mono text-[12.5px] text-accent-ink',
        className,
      )}
    >
      {children}
    </span>
  )
}
