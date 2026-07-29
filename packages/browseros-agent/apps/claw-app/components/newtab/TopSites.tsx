import { Globe } from 'lucide-react'
import { useState } from 'react'
import { siteOf } from '@/screens/cockpit/cockpit.helpers'
import { faviconUrl, type TopSite } from './top-sites.hooks'

interface TopSitesProps {
  topSites: TopSite[]
  pending: boolean
}

/** Calm favicon shortcut row. Hidden entirely on a fresh profile with no history. */
export function TopSites({ topSites, pending }: TopSitesProps) {
  if (pending) return <TopSitesSkeleton />
  if (topSites.length === 0) return null
  return (
    <nav
      aria-label="Top sites"
      className="flex flex-wrap items-start justify-center gap-1"
    >
      {topSites.map((site) => (
        <TopSiteTile key={site.url} site={site} />
      ))}
    </nav>
  )
}

function TopSiteTile({ site }: { site: TopSite }) {
  const [iconFailed, setIconFailed] = useState(false)
  const src = faviconUrl(site.url)
  const label = site.title.trim() || siteOf(site.url)
  return (
    <a
      href={site.url}
      title={label}
      data-top-site={site.url}
      className="group flex w-[84px] flex-col items-center gap-2 rounded-xl px-1.5 py-3 transition-colors hover:bg-card"
    >
      <span className="flex size-9 items-center justify-center rounded-full border border-border-2 bg-card shadow-card">
        {src && !iconFailed ? (
          <img
            src={src}
            alt=""
            className="size-5"
            onError={() => setIconFailed(true)}
          />
        ) : (
          <Globe aria-hidden className="size-4 text-ink-3" />
        )}
      </span>
      <span className="w-full truncate text-center text-[11px] text-ink-2">
        {label}
      </span>
    </a>
  )
}

function TopSitesSkeleton() {
  return (
    <div
      aria-hidden
      className="flex flex-wrap items-start justify-center gap-1"
    >
      {['s1', 's2', 's3', 's4', 's5', 's6', 's7', 's8'].map((id) => (
        <div
          key={id}
          className="flex w-[84px] flex-col items-center gap-2 px-1.5 py-3"
        >
          <span className="size-9 animate-pulse rounded-full bg-card-tint" />
          <span className="h-2.5 w-12 animate-pulse rounded bg-card-tint" />
        </div>
      ))}
    </div>
  )
}
