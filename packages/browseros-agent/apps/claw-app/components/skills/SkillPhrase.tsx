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

// Tailwind v4's rotate-*, scale-* and translate-* set the `rotate`, `scale` and `translate`
// properties, not `transform`, so a transition has to name them or the motion snaps.
const ICON_MOTION =
  'absolute inset-0 size-4 transition-[opacity,scale,rotate] duration-350 ease-[cubic-bezier(0.34,1.56,0.64,1)]'
const LABEL_MOTION =
  'transition-[opacity,translate] duration-250 ease-[cubic-bezier(0.16,1,0.3,1)] [grid-area:1/1]'

/**
 * The Copy cue's icon and label. Both pairs share one cell, so the cue keeps its width when it
 * turns into "Copied" and a check: the copy icon spins away as the check springs in, and the
 * label rolls up.
 */
export function CopyCueContent({ copied }: { copied: boolean }) {
  return (
    <>
      <span aria-hidden className="relative inline-block size-4 shrink-0">
        <Copy
          className={cn(ICON_MOTION, copied && 'rotate-45 scale-40 opacity-0')}
        />
        <Check
          strokeWidth={2.4}
          className={cn(
            ICON_MOTION,
            !copied && '-rotate-45 scale-40 opacity-0',
          )}
        />
      </span>
      <span className="inline-grid">
        <span
          className={cn(LABEL_MOTION, copied && '-translate-y-1.5 opacity-0')}
        >
          Copy
        </span>
        <span
          className={cn(LABEL_MOTION, !copied && 'translate-y-1.5 opacity-0')}
        >
          Copied
        </span>
      </span>
    </>
  )
}

/**
 * The line under a copy target: what to do before a copy, and what to do next after one. The two
 * share one cell and roll past each other. They are hidden from screen readers, which hear the
 * current line from a polite live region instead.
 */
export function CopiedLine({
  copied,
  before,
  after,
}: {
  copied: boolean
  before: string
  after: string
}) {
  return (
    <span className="inline-grid">
      <span
        aria-hidden
        className={cn(
          LABEL_MOTION,
          'text-ink-2',
          copied && '-translate-y-1.5 opacity-0',
        )}
      >
        {before}
      </span>
      <span
        aria-hidden
        className={cn(
          LABEL_MOTION,
          'font-semibold text-cyanotype-ink',
          !copied && 'translate-y-1.5 opacity-0',
        )}
      >
        {after}
      </span>
      <span aria-live="polite" className="sr-only">
        {copied ? after : before}
      </span>
    </span>
  )
}
