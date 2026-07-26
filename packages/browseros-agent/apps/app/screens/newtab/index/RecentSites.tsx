import { Globe } from 'lucide-react'
import { type FC, useState } from 'react'
import { getFavicons } from '@/lib/getFavicons'
import { useRecentSites } from './recent-sites.hooks'

const RecentSiteIcon: FC<{ src: string; alt: string }> = ({ src, alt }) => {
  const [failed, setFailed] = useState(false)

  if (failed) {
    return <Globe className="h-7 w-7 text-foreground" />
  }

  return (
    <img
      src={src}
      alt={alt}
      className="h-7 w-7 object-contain"
      onError={() => {
        setFailed(true)
      }}
      onLoad={(e) => {
        const img = e.currentTarget
        if (img.naturalWidth === 0 || img.naturalHeight === 0) {
          setFailed(true)
        }
      }}
    />
  )
}

export const RecentSites: FC = () => {
  const recentSites = useRecentSites()

  if (!recentSites.length) return null

  return (
    <div className="space-y-4">
      <h2 className="text-center font-medium text-muted-foreground text-xs uppercase tracking-wide">
        Recent sites
      </h2>
      <div className="flex flex-wrap items-center justify-center gap-6">
        {recentSites.map((site) => {
          const icon = site.host ? getFavicons(site.host) : undefined
          return (
            <a
              key={site.url}
              href={site.url}
              className="group flex flex-col items-center gap-2 transition-transform hover:scale-110"
            >
              <div className="flex h-14 w-14 items-center justify-center rounded-2xl border border-border/50 bg-card shadow-sm transition-all group-hover:border-[var(--accent-orange)]/30 group-hover:shadow-md">
                {icon ? (
                  <RecentSiteIcon src={icon} alt={site.name} />
                ) : (
                  <Globe className="h-7 w-7 text-foreground" />
                )}
              </div>
              <span className="line-clamp-1 max-w-18 text-muted-foreground text-xs transition-colors group-hover:text-foreground">
                {site.name}
              </span>
            </a>
          )
        })}
      </div>
    </div>
  )
}
