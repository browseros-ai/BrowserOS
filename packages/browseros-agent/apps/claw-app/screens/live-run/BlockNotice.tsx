import { Lock, Shield } from 'lucide-react'
import { Button } from '@/components/ui/button'
import type { PendingBlock } from '@/modules/api/run.hooks'

interface BlockNoticeProps {
  block: PendingBlock
  onManage: () => void
}

/**
 * Inline block notice rendered in the activity feed. There's a
 * sibling RunFlag that pins this same info just under the panel
 * header. Together they keep the block visible even after the feed
 * scrolls past the original action.
 */
export function BlockNotice({ block, onManage }: BlockNoticeProps) {
  return (
    <div className="overflow-hidden rounded-2xl border border-[#EBC4BF] bg-red-tint">
      <div className="p-4">
        <div className="mb-2 flex items-center gap-2">
          <span className="flex size-6 items-center justify-center rounded-md bg-card text-red">
            <Lock className="size-3.5" />
          </span>
          <span className="font-bold text-red text-sm">{block.title}</span>
        </div>
        <p className="mb-1 font-semibold text-ink text-sm">{block.detail}</p>
        <p className="mb-3 text-ink-2 text-xs leading-snug">{block.reason}</p>
        <Button
          type="button"
          variant="outline"
          onClick={onManage}
          className="border-[#E2A9A2] bg-card text-red hover:bg-red-tint/60 hover:text-red"
        >
          <Shield className="size-3.5" />
          Manage in Site Rules
        </Button>
      </div>
    </div>
  )
}
