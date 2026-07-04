import { Globe, StopCircle } from 'lucide-react'
import type { ReactNode } from 'react'
import { cn } from '@/lib/utils'

interface BrowserViewportProps {
  /** Site host shown in the chrome and center placeholder. */
  site: string
  /** Harness label shown in the agent badge. */
  harness: string
  /** When true the badge flips amber and the working pill is hidden. */
  paused: boolean
  /** Working-pill label rendered while the agent is actively doing something. */
  workingLabel?: string
  /** Hide the working pill (e.g. when run has paused or handed off). */
  hideWorkingPill?: boolean
  onStop: () => void
  /** Overlay content rendered above the page stub (e.g. the HandoffBanner). */
  overlay?: ReactNode
}

/**
 * Stub of the agent-driven browser tab. Renders fake browser chrome
 * (url bar with the run's site), a tinted page region with the host
 * called out, and the persistent agent badge + working pill from the
 * prototype. When a HandoffBanner is supplied via `overlay`, it sits
 * above the page stub and dims the rest.
 */
export function BrowserViewport({
  site,
  harness,
  paused,
  workingLabel,
  hideWorkingPill,
  onStop,
  overlay,
}: BrowserViewportProps) {
  return (
    <div className="relative flex min-w-0 flex-1 flex-col bg-card">
      <div className="flex h-9 shrink-0 items-center gap-2 border-border border-b bg-bg-sunken px-3">
        <span className="flex gap-1">
          <span className="size-2 rounded-full bg-[#FF5F57]" />
          <span className="size-2 rounded-full bg-[#FEBC2E]" />
          <span className="size-2 rounded-full bg-[#28C840]" />
        </span>
        <div className="ml-3 flex h-6 flex-1 items-center gap-2 rounded-md bg-card px-3 font-mono text-ink-3 text-xs">
          <Globe className="size-3" />
          <span className="truncate">{site}</span>
        </div>
      </div>
      <div className="relative flex-1 overflow-hidden">
        <div className="absolute inset-0 bg-[radial-gradient(circle_at_top,#fff,var(--color-bg-sunken))]" />
        <div className="absolute inset-0 flex flex-col items-center justify-center gap-3 text-ink-3">
          <Globe className="size-12" />
          <code className="font-mono text-sm">{site}</code>
          <p className="max-w-xs text-center text-ink-4 text-xs">
            Live tab view lands once the BrowserClaw Chromium recording pipeline
            is wired.
          </p>
        </div>
        <AgentBadge harness={harness} paused={paused} />
        {!hideWorkingPill && workingLabel && (
          <WorkingPill label={workingLabel} onStop={onStop} />
        )}
        {overlay}
      </div>
    </div>
  )
}

/* ---------------------------------------------------------------------------
 * Sub-components, kept private to the viewport.
 * -------------------------------------------------------------------------*/

function AgentBadge({ harness, paused }: { harness: string; paused: boolean }) {
  return (
    <div
      className={cn(
        'absolute top-3 right-4 z-30 flex items-center gap-2 rounded-full px-3 py-1.5 font-semibold text-white text-xs shadow-lg backdrop-blur',
        paused ? 'bg-[#B47814]/95' : 'bg-ink-deep/90',
      )}
    >
      <span
        aria-hidden
        className={cn(
          'size-1.5 rounded-full',
          paused ? 'bg-[#FFD66B]' : 'animate-pulse-dot bg-[#34D058]',
        )}
      />
      {paused ? "Paused. You're in control" : `${harness} is driving`}
    </div>
  )
}

function WorkingPill({ label, onStop }: { label: string; onStop: () => void }) {
  return (
    <div className="absolute bottom-5 left-1/2 z-30 flex -translate-x-1/2 items-center gap-3 overflow-hidden rounded-full bg-ink-deep/90 px-3 py-2 pl-4 shadow-2xl backdrop-blur">
      <span className="block size-4 shrink-0 animate-spin rounded-full border-2 border-[#8fb4ff]/30 border-t-[#8fb4ff]" />
      <span className="whitespace-nowrap font-semibold text-white/90 text-xs">
        {label}
      </span>
      <button
        type="button"
        onClick={onStop}
        title="Stop the agent"
        aria-label="Stop the agent"
        className="flex size-6 shrink-0 items-center justify-center rounded-lg bg-[#8fb4ff]/15 text-[#b8d3ff]"
      >
        <StopCircle className="size-3" />
      </button>
    </div>
  )
}
