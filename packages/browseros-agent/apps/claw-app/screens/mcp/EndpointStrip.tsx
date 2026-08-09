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
        <span className="text-[12px] text-cyanotype-muted">{label}</span>
        {hasValue && (
          <button
            type="button"
            onClick={copy}
            aria-label={`Copy ${label}`}
            className="group inline-flex items-center gap-1 text-[12px] text-cyanotype-blue transition-colors hover:text-cyanotype-blue-hover"
          >
            {copied ? 'Copied ✓' : 'Copy'}
            {!copied && (
              <span
                aria-hidden
                className="font-mono text-[10.5px] text-ink-3 tracking-[0.08em] transition-transform group-hover:translate-x-0.5"
              >
                →
              </span>
            )}
          </button>
        )}
      </div>
      <div className="overflow-hidden rounded-9 bg-cyanotype-blue px-4 py-3">
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
