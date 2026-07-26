import { useState } from 'react'

interface EndpointStripProps {
  label: string
  value: string | null
}

/** Renders an endpoint strip and hides copying until a resolved URL is available. */
export function EndpointStrip({ label, value }: EndpointStripProps) {
  const [copied, setCopied] = useState(false)
  const hasValue = value !== null
  const copy = async () => {
    if (value === null) return
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
        {hasValue && (
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
        )}
      </div>
      <div className="overflow-hidden rounded-xl bg-ink-deep px-4 py-3">
        {hasValue ? (
          <code
            className="block truncate font-mono text-[12.5px] text-white/95"
            title={value}
          >
            {value}
          </code>
        ) : (
          <div className="h-[18px] w-full max-w-sm animate-pulse rounded bg-white/15" />
        )}
      </div>
    </div>
  )
}
