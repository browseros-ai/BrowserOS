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
      <span className="text-[12px] text-cyanotype-muted">{label}</span>
      <div className="flex items-center gap-3 overflow-hidden rounded-9 bg-ink-deep px-4 py-3 shadow-card">
        {hasValue ? (
          <>
            <code
              className="min-w-0 flex-1 truncate font-mono text-[12.5px] text-white/90"
              title={value}
            >
              {value}
            </code>
            <button
              type="button"
              onClick={copy}
              aria-label={`Copy ${label}`}
              className="shrink-0 rounded px-1.5 py-0.5 text-[12px] text-white/70 transition-colors hover:bg-white/10 hover:text-white"
            >
              {copied ? 'Copied ✓' : 'Copy'}
            </button>
          </>
        ) : (
          <div className="h-[18px] w-full max-w-sm animate-pulse rounded bg-white/15" />
        )}
      </div>
    </div>
  )
}
