import { Check, Copy } from 'lucide-react'
import { useState } from 'react'
import { cn } from '@/lib/utils'

interface CommandBlockProps {
  command: string
  /** Fired once per successful copy, so the caller can track it. */
  onCopied?: () => void
  className?: string
}

/**
 * A shell command, shown in full and copyable.
 *
 * Deliberately not EndpointStrip: that one truncates to a single line, which
 * is right for a URL you only ever copy and wrong for a command someone is
 * meant to read and sanity-check before running.
 */
export function CommandBlock({
  command,
  onCopied,
  className,
}: CommandBlockProps) {
  const [copied, setCopied] = useState(false)

  const copy = async () => {
    try {
      await navigator.clipboard.writeText(command)
      setCopied(true)
      onCopied?.()
      window.setTimeout(() => setCopied(false), 1500)
    } catch {
      setCopied(false)
    }
  }

  return (
    <div
      className={cn(
        'flex items-start gap-3 rounded-xl border border-border-2 bg-bg-sunken px-3.5 py-3',
        className,
      )}
    >
      <code className="min-w-0 flex-1 whitespace-pre-wrap break-all font-mono text-[12.5px] text-ink leading-relaxed">
        {command}
      </code>
      <button
        type="button"
        onClick={copy}
        aria-label={copied ? 'Command copied' : 'Copy command'}
        className="inline-flex shrink-0 items-center gap-1.5 rounded-lg border border-border-2 px-2 py-1 text-[11.5px] text-ink-3 transition-colors hover:border-accent/40 hover:text-ink-2 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/40 active:scale-[0.98]"
      >
        {copied ? (
          <>
            <Check aria-hidden className="size-3.5 text-green" />
            Copied
          </>
        ) : (
          <>
            <Copy aria-hidden className="size-3.5" />
            Copy
          </>
        )}
      </button>
    </div>
  )
}
