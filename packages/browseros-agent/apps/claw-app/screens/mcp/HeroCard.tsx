import { Check, Copy, PlugZap } from 'lucide-react'
import { useState } from 'react'

interface HeroCardProps {
  url: string
  cli: string
}

/**
 * v2 hero card. One canonical URL, one CLI snippet, two copy
 * buttons. No per-agent slug, no regenerate flow. The URL block is
 * the headline; the CLI snippet is a fallback for harnesses without
 * a Connect button below or for users who prefer the manual install.
 */
export function HeroCard({ url, cli }: HeroCardProps) {
  return (
    <section className="rounded-2xl border border-border-2 bg-card p-6">
      <div className="flex items-start gap-3.5">
        <span className="flex size-11 shrink-0 items-center justify-center rounded-xl bg-accent-tint text-accent">
          <PlugZap className="size-5" />
        </span>
        <div className="flex-1">
          <div className="flex items-center gap-2">
            <h1 className="font-extrabold text-2xl tracking-tight">MCP</h1>
            <span className="inline-flex items-center gap-1.5 rounded-full bg-accent-tint px-2.5 py-0.5 font-bold text-accent-ink text-xs">
              1 endpoint
            </span>
          </div>
          <p className="mt-1 text-ink-2 text-sm leading-snug">
            Add BrowserOS as an MCP server in your AI agent. One endpoint, every
            harness. Use the buttons below to install with one click, or copy
            the URL for CI configs and manual scripts.
          </p>
        </div>
      </div>
      <div className="mt-5 space-y-2.5">
        <CopyBlock label="Endpoint URL" value={url} />
        <CopyBlock label="CLI snippet" value={cli} />
      </div>
    </section>
  )
}

interface CopyBlockProps {
  label: string
  value: string
}

function CopyBlock({ label, value }: CopyBlockProps) {
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
    <div className="space-y-1">
      <div className="font-semibold text-[11px] text-ink-3 uppercase tracking-wider">
        {label}
      </div>
      <div className="flex items-center gap-2 rounded-xl border border-border-strong bg-ink p-3 text-card">
        <code className="flex-1 truncate font-mono text-[12.5px]">{value}</code>
        <button
          type="button"
          onClick={copy}
          aria-label={`Copy ${label}`}
          className="flex size-7 shrink-0 items-center justify-center rounded-md text-ink-4 transition hover:bg-ink-2 hover:text-card"
        >
          {copied ? (
            <Check className="size-3.5" />
          ) : (
            <Copy className="size-3.5" />
          )}
        </button>
      </div>
    </div>
  )
}
