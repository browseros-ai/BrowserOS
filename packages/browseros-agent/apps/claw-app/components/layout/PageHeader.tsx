/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { ChevronLeft } from 'lucide-react'
import type { ReactNode } from 'react'
import { Link } from 'react-router'

export interface PageHeaderProps {
  title: ReactNode
  /** One short line (or a small block) under the title. */
  lede?: ReactNode
  /** A link back to the parent page, shown above the title. */
  back?: { to: string; label: string }
  /** A small row between the back link and the title, such as a status. */
  eyebrow?: ReactNode
  /** Controls aligned to the right of the title. */
  actions?: ReactNode
}

export function PageHeader({
  title,
  lede,
  back,
  eyebrow,
  actions,
}: PageHeaderProps) {
  return (
    <header className="flex flex-col gap-3">
      {back ? (
        <Link
          to={back.to}
          className="inline-flex w-fit items-center gap-1.5 font-medium text-ink-2 text-sm transition-colors hover:text-ink"
        >
          <ChevronLeft aria-hidden className="size-4" />
          {back.label}
        </Link>
      ) : null}
      {eyebrow ? (
        <div className="flex flex-wrap items-center gap-2.5">{eyebrow}</div>
      ) : null}
      <div className="flex flex-wrap items-start justify-between gap-4">
        <div className="flex min-w-0 flex-col gap-2">
          <h1 className="font-extrabold text-[26px] text-cyanotype-ink leading-[1.15] tracking-[-0.03em] md:text-[32px]">
            {title}
          </h1>
          {lede ? (
            <div className="max-w-[62ch] text-base text-ink-2">{lede}</div>
          ) : null}
        </div>
        {actions ? (
          <div className="flex shrink-0 items-center gap-2">{actions}</div>
        ) : null}
      </div>
    </header>
  )
}
