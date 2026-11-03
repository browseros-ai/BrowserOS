import { ExternalLink } from 'lucide-react'
import { ButtonLink } from '@/components/motion/button/base'
import { Card } from '@/components/motion/card'
import { SiteBadge } from '@/components/skills/SiteBadge'
import type { VisitedPage } from './task-detail.helpers'

/** The web pages a task went to, each with a way to open it again. */
export function PagesVisited({ pages }: { pages: VisitedPage[] }) {
  if (pages.length === 0) return null
  return (
    <section aria-labelledby="pages-title" className="flex flex-col gap-3.5">
      <h2
        id="pages-title"
        className="font-bold text-[19px] text-cyanotype-ink tracking-[-0.015em]"
      >
        Pages it visited
      </h2>
      <Card
        size="compact"
        className="gap-0 divide-y divide-border-2 overflow-hidden border-border-2 bg-card py-0 shadow-card"
      >
        {pages.map((page) => (
          <div
            key={page.url}
            className="flex items-center gap-3.5 px-4 py-3 sm:px-[18px]"
          >
            <SiteBadge site={page.url} className="size-8" />
            <span className="flex min-w-0 flex-1 flex-col">
              <span className="truncate font-semibold text-[15px] text-cyanotype-ink">
                {page.title}
              </span>
              <span className="truncate text-[13px] text-ink-3">
                {page.url.replace(/^https?:\/\/(www\.)?/, '')}
              </span>
            </span>
            <ButtonLink
              href={page.url}
              target="_blank"
              rel="noreferrer"
              variant="ghost"
              size="sm"
              aria-label={`Open ${page.title}`}
              className="h-8 shrink-0 font-semibold text-[13px]"
            >
              Open
              <ExternalLink aria-hidden className="size-[14px]" />
            </ButtonLink>
          </div>
        ))}
      </Card>
    </section>
  )
}
