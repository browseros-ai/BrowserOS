/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { cn } from '@/lib/utils'
import { Favicon } from './Favicon'

const SIZES = {
  sm: { chip: 'size-[22px] rounded-sm', icon: 'size-3.5 rounded-[3px]' },
  md: { chip: 'size-7 rounded-md', icon: 'size-4' },
  lg: { chip: 'size-9 rounded-md', icon: 'size-5' },
} as const

/**
 * A site's icon on a small white tile with a hairline, made to sit on top of a
 * page picture so the site still reads when the picture is a gray page.
 */
export function SiteChip({
  site,
  size = 'sm',
  className,
}: {
  site: string
  size?: keyof typeof SIZES
  className?: string
}) {
  const { chip, icon } = SIZES[size]
  return (
    <span
      aria-hidden
      className={cn(
        'grid shrink-0 place-items-center bg-card text-ink-3 shadow-[0_0_0_1px_var(--color-border-2),0_1px_3px_hsl(224_60%_20%/0.18)]',
        chip,
        className,
      )}
    >
      <Favicon site={site} className={icon} />
    </span>
  )
}
