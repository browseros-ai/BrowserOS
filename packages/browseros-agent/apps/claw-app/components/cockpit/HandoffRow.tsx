import { ArrowUpRight, User } from 'lucide-react'
import type { HandoffItem } from '@/modules/api/waiting.hooks'

interface HandoffRowProps {
  handoff: HandoffItem
}

/**
 * TODO(v2-restore-approvals): only rendered through WaitingStrip,
 * which is currently unwired in the v2 cockpit. Returns when the
 * permission gate ships.
 *
 * Inline "your turn" handoff row (CAPTCHA / MFA / security challenge).
 * Single action: Take over, which jumps the user into the live run
 * window where they finish the challenge in-place. Amber-themed to
 * match the design's HandoffRow.
 */
export function HandoffRow({ handoff }: HandoffRowProps) {
  return (
    <div className="overflow-hidden rounded-2xl border border-amber/30 bg-card shadow-card">
      <div className="h-1 bg-gradient-to-r from-amber to-amber/60" />
      <div className="flex flex-wrap items-center gap-4 px-5 py-4">
        <span className="flex size-9 shrink-0 items-center justify-center rounded-lg bg-amber-tint text-amber">
          <User className="size-4.5" />
        </span>
        <div className="min-w-[180px] flex-1">
          <div className="flex items-center gap-2">
            <span className="font-bold text-[11px] text-amber uppercase tracking-wider">
              Needs you
            </span>
            <span className="text-ink-3 text-xs">. {handoff.agentLabel}</span>
          </div>
          <div className="mt-0.5 font-bold text-sm">{handoff.title}</div>
          <div className="mt-0.5 text-ink-2 text-xs">
            {handoff.detail} .{' '}
            <span className="font-mono text-[11px]">{handoff.domain}</span>
          </div>
        </div>
        <button
          type="button"
          className="inline-flex items-center gap-1.5 whitespace-nowrap rounded-md bg-amber px-3.5 py-2 font-bold text-[13px] text-white transition hover:brightness-105"
        >
          Take over
          <ArrowUpRight className="size-3.5" />
        </button>
      </div>
    </div>
  )
}
