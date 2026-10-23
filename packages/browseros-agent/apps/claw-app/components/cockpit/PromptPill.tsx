/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { Check, Copy } from 'lucide-react'
import { useEffect, useRef, useState } from 'react'
import { cn } from '@/lib/utils'

const COPIED_MS = 1500

export interface PromptPillProps {
  prompt: string
  /** Fires once the prompt is on the clipboard (analytics, listening state). */
  onCopied?: () => void
}

/** A starter task that copies itself on click, then says so for 1.5 s. */
export function PromptPill({ prompt, onCopied }: PromptPillProps) {
  const [copied, setCopied] = useState(false)
  const timer = useRef<number | undefined>(undefined)
  useEffect(() => () => window.clearTimeout(timer.current), [])

  async function copy() {
    try {
      await navigator.clipboard.writeText(prompt)
    } catch {
      return
    }
    setCopied(true)
    onCopied?.()
    window.clearTimeout(timer.current)
    timer.current = window.setTimeout(() => setCopied(false), COPIED_MS)
  }

  return (
    <button
      type="button"
      onClick={copy}
      aria-label={`Copy: ${prompt}`}
      className="inline-flex max-w-full cursor-pointer items-center gap-2.5 rounded-xl border border-border bg-card px-3.5 py-2.5 text-left text-[15px] text-cyanotype-ink leading-5 transition-colors hover:border-border-strong hover:bg-bg-canvas focus-visible:outline-none focus-visible:ring-4 focus-visible:ring-accent/25"
    >
      <span>{prompt}</span>
      <span
        className={cn(
          'inline-flex flex-none items-center gap-[5px] font-semibold text-[13px]',
          copied ? 'text-accent' : 'text-ink-3',
        )}
      >
        {copied ? (
          <>
            Copied
            <Check aria-hidden className="size-[15px]" strokeWidth={2.5} />
          </>
        ) : (
          <Copy aria-hidden className="size-[15px]" />
        )}
      </span>
    </button>
  )
}
