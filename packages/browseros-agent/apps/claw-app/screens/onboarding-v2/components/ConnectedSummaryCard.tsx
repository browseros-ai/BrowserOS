import { CheckCircle2 } from 'lucide-react'

/**
 * Green success card shown after the fake connect flips. Phase 3
 * hard-codes the tool count and scope strings; the wiring person
 * threads real values from the connect response when they wire up
 * the connect mutation.
 */
export function ConnectedSummaryCard() {
  return (
    <div className="mb-[18px] flex animate-fade-up items-center gap-3 rounded-xl border border-green/30 bg-green-tint p-[18px]">
      <span className="flex size-[30px] items-center justify-center rounded-lg bg-card text-green">
        <CheckCircle2 className="size-[18px]" />
      </span>
      <div>
        <div className="font-bold text-[14px]">Connected to Claude</div>
        <div className="text-[12.5px] text-ink-2">
          68 browser tools available . scope: user
        </div>
      </div>
    </div>
  )
}
