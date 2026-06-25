import { Check, Copy } from 'lucide-react'
import { useState } from 'react'

interface OnboardingCopyBlockProps {
  text: string
}

/**
 * Dark code block with a copy button matching the v2 onboarding
 * design's CLI snippet shape (green prompt prefix, light mono text,
 * inline copy chip that flashes a checkmark for ~1.5 seconds). The
 * MCP page's `HeroCard` has a sibling block; once Phase 3 merges the
 * two can collapse into a shared `components/code/CopyBlock`, but
 * keeping them parallel during the Phase 3 window avoids a merge
 * conflict against Phase 2's just-landed code.
 */
export function OnboardingCopyBlock({ text }: OnboardingCopyBlockProps) {
  const [copied, setCopied] = useState(false)
  const copy = async () => {
    try {
      await navigator.clipboard.writeText(text)
      setCopied(true)
      window.setTimeout(() => setCopied(false), 1500)
    } catch {
      setCopied(false)
    }
  }

  return (
    <div className="flex items-center gap-2.5 rounded-xl bg-[#15140F] px-3.5 py-3">
      <span className="font-mono text-[#6FCF8E] text-[12.5px]">$</span>
      <code className="flex-1 truncate font-mono text-[#EDEAE2] text-[12.5px]">
        {text}
      </code>
      <button
        type="button"
        onClick={copy}
        className="inline-flex items-center gap-1.5 rounded-md bg-white/10 px-2 py-1 font-semibold text-[11.5px] text-white transition hover:bg-white/15"
      >
        {copied ? (
          <Check className="size-3.5" />
        ) : (
          <Copy className="size-3.5" />
        )}
        {copied ? 'Copied' : 'Copy'}
      </button>
    </div>
  )
}
