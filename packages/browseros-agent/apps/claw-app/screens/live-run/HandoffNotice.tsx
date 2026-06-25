import { Check, StopCircle, User } from 'lucide-react'
import { Button } from '@/components/ui/button'
import type { PendingHandoff } from '@/modules/api/run.hooks'

interface HandoffNoticeProps {
  handoff: PendingHandoff
  onContinue: () => void
  onCancel: () => void
}

/**
 * In-panel "Your turn" card. Mirrors the page-level HandoffBanner so
 * the user can resume from either surface. Clicking Continue picks
 * the run back up; Cancel aborts the run.
 */
export function HandoffNotice({
  handoff,
  onContinue,
  onCancel,
}: HandoffNoticeProps) {
  return (
    <div className="overflow-hidden rounded-2xl border border-[#E7D3A6] bg-amber-tint">
      <div className="h-[3px] bg-gradient-to-r from-[#C98A1B] to-[#E0A93B]" />
      <div className="p-4">
        <div className="mb-2 flex items-center gap-2">
          <span className="flex size-6 items-center justify-center rounded-md bg-card text-amber">
            <User className="size-3.5" />
          </span>
          <span className="font-bold text-[10.5px] text-amber uppercase tracking-wider">
            Your turn
          </span>
        </div>
        <h2 className="mb-1 font-bold text-ink text-sm leading-snug">
          {handoff.title}
        </h2>
        <p className="mb-3 text-ink-2 text-xs leading-snug">
          Finish it in the live window, then continue. {handoff.note}
        </p>
        <div className="flex gap-1.5">
          <Button type="button" onClick={onContinue} className="flex-1">
            <Check className="size-3.5" />
            Continue
          </Button>
          <Button type="button" variant="destructive" onClick={onCancel}>
            <StopCircle className="size-3.5" />
            Cancel run
          </Button>
        </div>
      </div>
    </div>
  )
}
