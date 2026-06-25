import { Check, Copy, Sparkles } from 'lucide-react'
import { useState } from 'react'
import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'
import { STARTER_PROMPTS } from '@/modules/api/onboarding.hooks'

interface ReadyStepProps {
  onDone: () => void
}

/**
 * Step 3. Two starter prompts the user can copy into Claude plus the
 * "Open BrowserOS" CTA that finishes onboarding.
 */
export function ReadyStep({ onDone }: ReadyStepProps) {
  return (
    <div className="flex w-full max-w-[560px] flex-col">
      <h1 className="mb-3 font-extrabold text-[34px] text-ink leading-[1.05] tracking-tight">
        You're{' '}
        <em className="font-['Newsreader',serif] font-medium text-accent italic">
          set
        </em>
        .
      </h1>
      <p className="mb-5 max-w-[470px] text-ink-2 text-sm leading-relaxed">
        Open Claude and try one of these. The task runs here, in BrowserOS: you
        watch, approve, and audit.
      </p>
      <div className="mb-6 flex flex-col gap-2.5">
        {STARTER_PROMPTS.slice(0, 2).map((prompt) => (
          <StarterPromptRow key={prompt} prompt={prompt} />
        ))}
      </div>
      <Button type="button" size="lg" onClick={onDone} className="self-start">
        <Sparkles className="size-4" />
        Open BrowserOS
      </Button>
    </div>
  )
}

/* ---------------------------------------------------------------------------
 * Sub-components, private to this step.
 * -------------------------------------------------------------------------*/

function StarterPromptRow({ prompt }: { prompt: string }) {
  const [copied, setCopied] = useState(false)
  const onCopy = async () => {
    try {
      await navigator.clipboard.writeText(prompt)
      setCopied(true)
      window.setTimeout(() => setCopied(false), 1500)
    } catch {
      setCopied(false)
    }
  }
  return (
    <div className="flex items-center gap-3 rounded-xl border border-border-2 bg-card p-3.5">
      <Sparkles className="size-4 shrink-0 text-accent" />
      <span className="min-w-0 flex-1 text-ink text-sm">{prompt}</span>
      <Button
        type="button"
        variant="outline"
        size="sm"
        onClick={onCopy}
        className={cn('h-8 gap-1.5 px-2.5 text-xs', copied && 'text-green')}
      >
        {copied ? <Check className="size-3" /> : <Copy className="size-3" />}
        {copied ? 'Copied' : 'Copy'}
      </Button>
    </div>
  )
}
