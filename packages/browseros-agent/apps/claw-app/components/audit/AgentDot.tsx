import { cn } from '@/lib/utils'
import { hexForSlug } from '@/screens/audit/audit.colors'

interface AgentDotProps {
  slug: string
  className?: string
}

export function AgentDot({ slug, className }: AgentDotProps) {
  return (
    <span
      className={cn('inline-block size-2 rounded-full', className)}
      style={{ background: hexForSlug(slug) }}
      aria-hidden
    />
  )
}
