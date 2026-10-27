import { Globe } from 'lucide-react'
import { cn } from '@/lib/utils'

/** The square that stands for the website a skill works on. */
export function SiteBadge({ className }: { className?: string }) {
  return (
    <span
      aria-hidden
      className={cn(
        'grid size-10 shrink-0 place-items-center rounded-[10px] border border-border-2 bg-bg-sunken text-ink-3',
        className,
      )}
    >
      <Globe className="size-[18px]" />
    </span>
  )
}
