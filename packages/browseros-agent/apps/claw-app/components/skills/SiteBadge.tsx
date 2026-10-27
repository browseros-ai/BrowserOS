import { Globe } from 'lucide-react'
import { useState } from 'react'
import { cn } from '@/lib/utils'
import { faviconUrl } from './site-badge.helpers'

/** The square that stands for the website a skill works on: its icon, or a globe. */
export function SiteBadge({
  site,
  className,
}: {
  site?: string
  className?: string
}) {
  const src = site ? faviconUrl(site) : null
  return (
    <span
      aria-hidden
      className={cn(
        'grid size-10 shrink-0 place-items-center rounded-[10px] border border-border-2 bg-bg-sunken text-ink-3',
        className,
      )}
    >
      {src ? (
        <SiteIcon key={src} src={src} />
      ) : (
        <Globe className="size-[18px]" />
      )}
    </span>
  )
}

function SiteIcon({ src }: { src: string }) {
  const [failed, setFailed] = useState(false)
  if (failed) return <Globe className="size-[18px]" />
  return (
    <img
      src={src}
      alt=""
      className="size-[18px] rounded-[4px]"
      onError={() => setFailed(true)}
    />
  )
}
