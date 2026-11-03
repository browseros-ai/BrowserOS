import { Globe } from 'lucide-react'
import { Favicon } from '@/components/favicon/Favicon'
import { cn } from '@/lib/utils'

/** The square that stands for the website a skill works on: its icon, or a globe. */
export function SiteBadge({
  site,
  className,
}: {
  site?: string
  className?: string
}) {
  return (
    <span
      aria-hidden
      className={cn(
        'grid size-10 shrink-0 place-items-center rounded-md border border-border-2 bg-bg-sunken text-ink-3',
        className,
      )}
    >
      {site ? <Favicon site={site} /> : <Globe className="size-[18px]" />}
    </span>
  )
}
