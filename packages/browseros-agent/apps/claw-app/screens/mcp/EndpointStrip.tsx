import { useState } from 'react'

interface EndpointStripProps {
  label: string
  value: string
}

/**
 * Editorial endpoint strip. Mono uppercase label on top with an
 * inline `copy →` link, dark-ink `bg-ink` strip below carrying the
 * value in mono white. Copies to clipboard and flips the link to
 * `copied ✓` for 1.5 s. Long values `truncate`; the native `title`
 * attribute reveals the full string on hover.
 */
export function EndpointStrip({ label, value }: EndpointStripProps) {
  const [copied, setCopied] = useState(false)
  const copy = async () => {
    try {
      await navigator.clipboard.writeText(value)
      setCopied(true)
      window.setTimeout(() => setCopied(false), 1500)
    } catch {
      setCopied(false)
    }
  }
  return (
    <div className="space-y-2">
      <div className="flex items-baseline justify-between gap-3">
        <span className="font-mono text-[10.5px] text-ink-3 uppercase tracking-[0.08em]">
          {label}
        </span>
        <button
          type="button"
          onClick={copy}
          aria-label={`Copy ${label}`}
          className="group inline-flex items-center gap-1 font-mono text-[10.5px] text-ink-3 uppercase tracking-[0.08em] transition-colors hover:text-accent"
        >
          {copied ? 'copied ✓' : 'copy'}
          {!copied && (
            <span
              aria-hidden
              className="transition-transform group-hover:translate-x-0.5"
            >
              →
            </span>
          )}
        </button>
      </div>
      <div className="overflow-hidden rounded-xl bg-ink px-4 py-3">
        <code
          className="block truncate font-mono text-[12.5px] text-white/95"
          title={value}
        >
          {value}
        </code>
      </div>
    </div>
  )
}
