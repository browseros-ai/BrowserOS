import { Check, Compass, Globe, Send, Shield, Sparkles, X } from 'lucide-react'
import type { ComponentType, SVGProps } from 'react'
import { Button } from '@/components/ui/button'
import type { PendingApproval } from '@/modules/api/run.hooks'

interface ApprovalCardProps {
  approval: PendingApproval
  harnessLabel: string
  onAllowOnce: () => void
  onAllowAlways: () => void
  onBlock: () => void
}

const KIND_ICON: Record<
  PendingApproval['kind'],
  ComponentType<SVGProps<SVGSVGElement>>
> = {
  navigate: Compass,
  submit: Send,
  payment: Send,
  delete: X,
  upload: Send,
}

const ACTION_WORD: Record<PendingApproval['kind'], string> = {
  navigate: 'visits to',
  submit: 'submits on',
  payment: 'payments on',
  delete: 'deletes on',
  upload: 'uploads on',
}

/**
 * The approval card pinned inside the activity panel. Mirrors the
 * scope line and three-button shape the v1 UX spec calls out as
 * non-negotiable: "this permission applies to <domain> only".
 */
export function ApprovalCard({
  approval,
  harnessLabel,
  onAllowOnce,
  onAllowAlways,
  onBlock,
}: ApprovalCardProps) {
  const KindIcon = KIND_ICON[approval.kind] ?? Send
  return (
    <div className="overflow-hidden rounded-2xl border border-accent-tint-2 bg-card shadow-md">
      <div className="h-[3px] bg-gradient-to-r from-accent to-accent-2" />
      <div className="p-4">
        <div className="mb-2.5 flex items-center gap-2">
          <span className="flex size-6 items-center justify-center rounded-md bg-accent-tint text-accent">
            <KindIcon className="size-3.5" />
          </span>
          <span className="font-bold text-[10.5px] text-amber uppercase tracking-wider">
            Approval needed
          </span>
          <span className="flex-1" />
          <span className="inline-flex items-center gap-1.5 text-ink-3 text-xs">
            <span className="flex size-3.5 items-center justify-center rounded-[4px] bg-accent">
              <Sparkles className="size-2.5 text-card" />
            </span>
            {harnessLabel}
          </span>
        </div>
        <h2 className="mb-2.5 font-bold text-[15px] text-ink leading-snug">
          {approval.title}
        </h2>
        <div className="mb-2.5 rounded-lg bg-bg-sunken px-3 py-2.5">
          <div className="mb-1 font-bold text-[10px] text-ink-3 uppercase tracking-wider">
            Action
          </div>
          <div className="font-semibold text-ink text-sm leading-snug">
            {approval.detail}
          </div>
          <div className="mt-2 flex items-center gap-1.5 text-ink-2 text-xs">
            <Globe className="size-3 text-ink-3" />
            <span className="font-mono">{approval.domain}</span>
          </div>
        </div>
        <div className="mb-3 flex items-start gap-2 text-ink-2">
          <Shield className="mt-0.5 size-3.5 shrink-0 text-ink-3" />
          <p className="text-xs leading-snug">{approval.scope}</p>
        </div>
        <div className="flex flex-col gap-1.5">
          <Button type="button" onClick={onAllowOnce} className="w-full">
            <Check className="size-3.5" />
            Allow once
          </Button>
          <div className="flex gap-1.5">
            <Button
              type="button"
              variant="outline"
              onClick={onAllowAlways}
              className="flex-1 text-xs"
            >
              Always allow {ACTION_WORD[approval.kind]} {approval.domain}
            </Button>
            <Button
              type="button"
              variant="destructive"
              onClick={onBlock}
              className="shrink-0"
            >
              <X className="size-3.5" />
              Block
            </Button>
          </div>
        </div>
      </div>
    </div>
  )
}
