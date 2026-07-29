import { Globe } from 'lucide-react'
import { Skeleton } from '@/components/ui/skeleton'
import { siteOf } from '@/screens/cockpit/cockpit.helpers'
import { faviconUrl } from './top-sites.helpers'
import type { TopSite } from './top-sites.hooks'

interface TopSitesProps {
  sites: TopSite[]
  isPending: boolean
}

const SKELETON_KEYS = ['a', 'b', 'c', 'd', 'e', 'f', 'g', 'h']

/**
 * The centered favicon shortcut row. Hidden entirely on a fresh profile with
 * no history so the focal stays calm rather than showing empty placeholders.
 */
export function TopSites({ sites, isPending }: TopSitesProps) {
  if (isPending) {
    return (
      <div
        aria-hidden
        className="flex flex-wrap items-start justify-center gap-1"
      >
        {SKELETON_KEYS.map((key) => (
          <div
            className="flex w-[76px] flex-col items-center gap-1.5 px-2 py-2"
            key={key}
          >
            <Skeleton className="size-8 rounded-lg" />
            <Skeleton className="h-2.5 w-12" />
          </div>
        ))}
      </div>
    )
  }

  if (sites.length === 0) return null

  return (
    <nav
      aria-label="Top sites"
      className="flex flex-wrap items-start justify-center gap-1"
    >
      {sites.map((site) => (
        <TopSiteTile key={site.url} site={site} />
      ))}
    </nav>
  )
}

function TopSiteTile({ site }: { site: TopSite }) {
  const label = site.title || siteOf(site.url)
  const icon = faviconUrl(site.url)

  return (
    <a
      className="group flex w-[76px] flex-col items-center gap-1.5 rounded-xl px-2 py-2 transition-colors hover:bg-card-tint"
      href={site.url}
      title={label}
    >
      <span className="relative flex size-8 items-center justify-center overflow-hidden rounded-lg border border-border-2 bg-card">
        <Globe aria-hidden className="size-4 text-ink-3" />
        {icon ? (
          <img
            alt=""
            className="absolute inset-0 size-full object-contain p-1"
            loading="lazy"
            // A failed favicon reveals the globe fallback painted underneath.
            onError={(event) => {
              event.currentTarget.style.display = 'none'
            }}
            src={icon}
          />
        ) : null}
      </span>
      <span className="w-full truncate text-center text-[11px] text-ink-2">
        {label}
      </span>
    </a>
  )
}
