import { ChevronRight, Send, X } from 'lucide-react'
import { Button } from '@/components/ui/button'
import type { ApprovalItem } from '@/modules/api/waiting.hooks'

interface ApprovalBannerProps {
  approval: ApprovalItem
}

/**
 * TODO(v2-restore-approvals): only rendered through WaitingStrip,
 * which is currently unwired in the v2 cockpit. Returns when the
 * permission gate ships.
 *
 * Inline "approval needed" card. Three actions: allow once, always
 * allow (scoped), block. Matches dashboard.jsx's ApprovalBanner: a
 * 3px accent gradient strip on top, then a row of agent label,
 * approval summary, and the action cluster. Open-run jump-link kept
 * for parity even though the run surface is not built yet.
 */
export function ApprovalBanner({ approval }: ApprovalBannerProps) {
  return (
    <div className="overflow-hidden rounded-2xl border border-accent-tint-2 bg-card shadow-card">
      <div className="h-1 bg-gradient-to-r from-accent to-accent-2" />
      <div className="flex flex-wrap items-center gap-4 px-5 py-4">
        <span className="flex size-9 shrink-0 items-center justify-center rounded-lg bg-accent-tint text-accent">
          <Send className="size-4.5" />
        </span>
        <div className="min-w-[180px] flex-1">
          <div className="flex items-center gap-2">
            <span className="font-bold text-[11px] text-amber uppercase tracking-wider">
              Approval needed
            </span>
            <span className="text-ink-3 text-xs">. {approval.agentLabel}</span>
          </div>
          <div className="mt-0.5 font-bold text-sm">{approval.title}</div>
          <div className="mt-0.5 text-ink-2 text-xs">
            {approval.detail} .{' '}
            <span className="font-mono text-[11px]">{approval.domain}</span>
          </div>
        </div>
        <div className="flex items-center gap-2">
          <Button size="sm">Allow once</Button>
          <Button size="sm" variant="ghost">
            Always allow
          </Button>
          <Button size="sm" variant="destructive">
            <X className="mr-1 size-3.5" /> Block
          </Button>
          <Button
            size="sm"
            variant="ghost"
            className="text-accent-ink hover:text-accent-ink"
          >
            Open run
            <ChevronRight className="ml-0.5 size-3.5" />
          </Button>
        </div>
      </div>
    </div>
  )
}
