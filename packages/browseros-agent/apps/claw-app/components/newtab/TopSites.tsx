import { Globe } from 'lucide-react'
import { useState } from 'react'
import { siteOf } from '@/screens/cockpit/cockpit.helpers'
import { faviconUrl, type TopSite } from './top-sites.hooks'

interface TopSitesProps {
  sites: TopSite[]
  isPending: boolean
}

/**
 * Calm row of most-visited shortcuts. Hidden entirely on a fresh profile with
 * no history rather than showing placeholder tiles.
 */
export function TopSites({ sites, isPending }: TopSitesProps) {
  if (isPending) {
    return (
      <div className="flex flex-wrap justify-center gap-2">
        {['s1', 's2', 's3', 's4', 's5', 's6'].map((id) => (
          <div
            key={id}
            className="h-9 w-32 animate-pulse rounded-full bg-card-tint"
          />
        ))}
      </div>
    )
  }

  if (sites.length === 0) return null

  return (
    <nav aria-label="Top sites" className="flex flex-wrap justify-center gap-2">
      {sites.map((site) => (
        <TopSiteTile key={site.url} site={site} />
      ))}
    </nav>
  )
}

function TopSiteTile({ site }: { site: TopSite }) {
  const [iconFailed, setIconFailed] = useState(false)
  const icon = faviconUrl(site.url)
  const label = site.title || siteOf(site.url)

  return (
    <a
      href={site.url}
      title={label}
      className="group inline-flex max-w-[13rem] items-center gap-2 rounded-full border border-border-2 bg-card px-3 py-2 text-ink-2 text-sm shadow-card transition-colors hover:border-accent/40 hover:bg-card-tint"
    >
      {icon && !iconFailed ? (
        <img
          src={icon}
          alt=""
          aria-hidden
          className="size-4 shrink-0 rounded-sm"
          onError={() => setIconFailed(true)}
        />
      ) : (
        <Globe aria-hidden className="size-4 shrink-0 text-ink-3" />
      )}
      <span className="truncate">{label}</span>
    </a>
  )
}
