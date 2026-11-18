/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { cn } from '@/lib/utils'

/** "ultrafast" in its bold italic Signal Blue: the brand, used once per surface as a label. */
export function UltrafastWordmark({ className }: { className?: string }) {
  return (
    <span
      className={cn(
        'whitespace-nowrap font-extrabold text-accent-ink italic tracking-[-0.01em]',
        className,
      )}
    >
      ultrafast
    </span>
  )
}

/** "ultrafast" in its bold italic, then what ultrafast did, in words. */
export function UltrafastNote({
  children,
  className,
}: {
  children: string
  className?: string
}) {
  return (
    <span className={cn('text-[13.5px] text-ink-2', className)}>
      <UltrafastWordmark className="mr-1.5" />
      {children}
    </span>
  )
}
