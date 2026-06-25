import { Check, CreditCard, Lock } from 'lucide-react'

interface ImportedSummaryCardProps {
  sites: number
  profileCount: number
}

/**
 * Success card shown after the fake import flips. Headline + three
 * bullet rows mirroring the v2 design's success state. Phase 3 leaves
 * the payment-cards-skipped count hard-coded; the wiring person
 * threads the real value when they wire up the import service.
 */
export function ImportedSummaryCard({
  sites,
  profileCount,
}: ImportedSummaryCardProps) {
  return (
    <div className="mb-[18px] animate-fade-up rounded-xl border border-border-2 bg-card p-[18px]">
      <div className="mb-3 flex items-center gap-2.5">
        <span className="flex size-7 items-center justify-center rounded-lg bg-green-tint text-green">
          <Check className="size-4" />
        </span>
        <span className="font-bold text-[14px]">
          Imported {sites} sites from {profileCount} profile
          {profileCount === 1 ? '' : 's'}
        </span>
      </div>
      <SummaryRow
        icon={<Check className="size-3.5 text-green" />}
        text={`${sites} logged-in sessions ready`}
      />
      <SummaryRow
        icon={<Lock className="size-3.5 text-ink-3" />}
        text="Passwords stored in vault. Never shown to you or the agent."
      />
      <SummaryRow
        icon={<CreditCard className="size-3.5 text-ink-3" />}
        text="3 payment cards skipped"
      />
    </div>
  )
}

interface SummaryRowProps {
  icon: React.ReactNode
  text: string
}

function SummaryRow({ icon, text }: SummaryRowProps) {
  return (
    <div className="flex items-center gap-2.5 py-1 text-[12.5px] text-ink-2">
      {icon}
      {text}
    </div>
  )
}
