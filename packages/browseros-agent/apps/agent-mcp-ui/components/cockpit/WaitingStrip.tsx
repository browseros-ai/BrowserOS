import { AlertTriangle } from 'lucide-react'
import type { ApprovalItem, HandoffItem } from '@/modules/api/waiting.hooks'
import { ApprovalBanner } from './ApprovalBanner'
import { HandoffRow } from './HandoffRow'

interface WaitingStripProps {
  approvals: ApprovalItem[]
  handoffs: HandoffItem[]
}

/**
 * Sticky-attention strip surfaced above the running grid. Renders
 * nothing when both arrays are empty (no count chip, no header). The
 * count chip on the header is bright amber so it draws the eye
 * regardless of how saturated the running grid gets below.
 */
export function WaitingStrip({ approvals, handoffs }: WaitingStripProps) {
  const total = approvals.length + handoffs.length
  if (total === 0) return null

  return (
    <section className="space-y-3">
      <div className="flex items-center gap-2.5">
        <h2 className="font-bold text-base">Waiting on you</h2>
        <span className="inline-flex items-center gap-1 rounded-full bg-amber-tint px-2 py-0.5 font-bold text-[11px] text-amber">
          <AlertTriangle className="size-3" />
          {total}
        </span>
      </div>
      <div className="space-y-3">
        {handoffs.map((h) => (
          <HandoffRow key={h.id} handoff={h} />
        ))}
        {approvals.map((a) => (
          <ApprovalBanner key={a.id} approval={a} />
        ))}
      </div>
    </section>
  )
}
