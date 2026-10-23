/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { cn } from '@/lib/utils'

/** neo's dog mark, drawn from the shipped 128 px icon (crisp up to 96 px). */
export function NeoMark({ className }: { className?: string }) {
  // Must stay a literal public path: importing the asset or inlining it
  // breaks scripts/verify-chromium-build.ts (allowlist, no data: URLs).
  return (
    <img
      alt=""
      aria-hidden
      className={cn('shrink-0 select-none', className)}
      draggable={false}
      src="/icon/128.png"
    />
  )
}
