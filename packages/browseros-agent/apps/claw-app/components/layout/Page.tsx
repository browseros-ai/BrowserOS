/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { ReactNode } from 'react'
import { cn } from '@/lib/utils'
import { PAGE_MAX_WIDTH } from './layout.constants'

export interface PageProps {
  children: ReactNode
  className?: string
}

/**
 * The frame every screen renders into: one width, one set of gutters and one
 * vertical rhythm, so moving between pages never shifts the content edge.
 * Screens never set their own outer width or padding.
 */
export function Page({ children, className }: PageProps) {
  return (
    <div
      data-page
      style={{ maxWidth: PAGE_MAX_WIDTH }}
      className={cn(
        'mx-auto flex w-full flex-col gap-7 px-4 pt-7 pb-[120px] md:gap-9 md:px-10 md:pt-12',
        className,
      )}
    >
      {children}
    </div>
  )
}
