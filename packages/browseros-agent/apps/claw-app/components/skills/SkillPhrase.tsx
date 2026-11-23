/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { Check, Copy } from 'lucide-react'
import { cn } from '@/lib/utils'

/**
 * "use the neo-x skill", wrapping only between words. The skill name is one unbreakable piece in
 * a heavier weight, so the eye lands on it; it ellipsises only when it is wider than the line.
 */
export function SkillPhrase({
  name,
  before = 'use the',
  after = 'skill',
  className,
}: {
  name: string
  before?: string
  after?: string
  className?: string
}) {
  return (
    <span
      className={cn(
        'min-w-0 font-mono text-[color-mix(in_srgb,var(--accent-ink)_70%,var(--ink-3))] tracking-[-0.01em]',
        className,
      )}
    >
      {before ? `${before} ` : null}
      <span className="inline-block max-w-full truncate align-bottom font-semibold text-accent-ink">
        {name}
      </span>
      {after ? ` ${after}` : null}
    </span>
  )
}

/**
 * The Copy cue's icon and label. Both pairs share one cell, so the cue keeps its width when it
 * turns into "Copied" and a check.
 */
export function CopyCueContent({ copied }: { copied: boolean }) {
  return (
    <>
      <span aria-hidden className="relative inline-block size-4 shrink-0">
        <Copy
          className={cn(
            'absolute inset-0 size-4 transition-[opacity,transform] duration-300 motion-reduce:transition-none',
            copied && 'rotate-45 scale-50 opacity-0',
          )}
        />
        <Check
          strokeWidth={2.4}
          className={cn(
            'absolute inset-0 size-4 transition-[opacity,transform] duration-300 motion-reduce:transition-none',
            !copied && '-rotate-45 scale-50 opacity-0',
          )}
        />
      </span>
      <span className="inline-grid">
        <span
          className={cn(
            'transition-[opacity,transform] duration-200 [grid-area:1/1] motion-reduce:transition-none',
            copied && '-translate-y-1.5 opacity-0',
          )}
        >
          Copy
        </span>
        <span
          className={cn(
            'transition-[opacity,transform] duration-200 [grid-area:1/1] motion-reduce:transition-none',
            !copied && 'translate-y-1.5 opacity-0',
          )}
        >
          Copied
        </span>
      </span>
    </>
  )
}
