import { Check, Copy, Sparkles } from 'lucide-react'
import { useState } from 'react'

interface StarterPromptTileProps {
  prompt: string
}

/**
 * Suggested starter prompt row. Clicking Copy puts the prompt on the
 * clipboard and flashes a checkmark for ~1.5 seconds, matching the
 * MCP page's CopyBlock pattern.
 */
export function StarterPromptTile({ prompt }: StarterPromptTileProps) {
  const [copied, setCopied] = useState(false)
  const copy = async () => {
    try {
      await navigator.clipboard.writeText(prompt)
      setCopied(true)
      window.setTimeout(() => setCopied(false), 1500)
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
