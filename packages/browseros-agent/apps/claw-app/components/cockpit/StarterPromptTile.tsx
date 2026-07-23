/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * The copyable "try this prompt" tile on the cockpit first-run
 * block. One click writes the starter prompt to the clipboard and
 * calls back so the surrounding UI can flip the waiting banner to
 * "listening" without waiting for the polling probe.
 */

import { Check, Copy, Sparkles } from 'lucide-react'
import { useState } from 'react'

interface StarterPromptTileProps {
  prompt: string
  /**
   * Fires after the prompt lands on the clipboard. The parent uses
   * this to bump analytics + show the waiting banner immediately
   * rather than waiting for the next session poll.
   */
  onCopied?: () => void
}

export function StarterPromptTile({
  prompt,
  onCopied,
}: StarterPromptTileProps) {
  const [copied, setCopied] = useState(false)
  const copy = async () => {
    try {
      await navigator.clipboard.writeText(prompt)
      setCopied(true)
      onCopied?.()
      window.setTimeout(() => setCopied(false), 1800)
    } catch {
      setCopied(false)
    }
  }
  return (
    <div className="flex items-center gap-3 rounded-xl border border-border-2 bg-card px-4 py-3.5">
      <Sparkles className="size-4 shrink-0 text-accent" />
      <span className="flex-1 text-[13.5px] text-ink">{prompt}</span>
      <button
        type="button"
        onClick={copy}
        className="inline-flex shrink-0 items-center gap-1.5 rounded-md bg-bg-sunken px-2.5 py-1 font-semibold text-[12px] text-ink-2 transition hover:bg-card-tint hover:text-ink"
      >
        {copied ? <Check className="size-3" /> : <Copy className="size-3" />}
        {copied ? 'Copied' : 'Copy'}
      </button>
    </div>
  )
}
