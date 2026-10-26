import type { SessionSummary } from '@browseros/claw-api'
import { ChevronDown, Wrench } from 'lucide-react'
import type { ReactNode } from 'react'
import { CopyButton } from '@/components/copy/CopyButton'
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleIndicator,
  CollapsibleTrigger,
} from '@/components/motion/collapsible'

interface TechnicalDetailsProps {
  session: SessionSummary
  dispatchCount: number
  /** Extra controls, such as the full step log. */
  children?: ReactNode
}

const dateTime = (ms: number) =>
  new Date(ms).toLocaleString(undefined, {
    dateStyle: 'medium',
    timeStyle: 'medium',
  })

/** The ids and numbers support may ask for, closed until someone wants them. */
export function TechnicalDetails({
  session,
  dispatchCount,
  children,
}: TechnicalDetailsProps) {
  const tokens = session.tokenUsage?.totalTokenEstimate
  const rows: [string, string][] = [
    ['Task id', session.sessionId],
    ['Name your AI gave it', session.slug],
    ['Steps recorded', dispatchCount.toLocaleString()],
    ['Tokens used', tokens != null ? tokens.toLocaleString() : 'Not measured'],
    ['Started', dateTime(session.startedAt)],
    ['Ended', session.endedAt ? dateTime(session.endedAt) : 'Still running'],
  ]

  return (
    <Collapsible className="rounded-2xl border border-border-2 bg-card shadow-card">
      <CollapsibleTrigger className="flex w-full items-center gap-2.5 rounded-2xl px-[18px] py-3.5 text-left font-semibold text-[15px] text-ink-2 outline-none hover:text-cyanotype-ink focus-visible:ring-2 focus-visible:ring-ring">
        <Wrench aria-hidden className="size-[15px]" />
        <span className="flex-1">Technical details</span>
        <CollapsibleIndicator>
          <ChevronDown aria-hidden className="size-4" />
        </CollapsibleIndicator>
      </CollapsibleTrigger>
      <CollapsibleContent contentClassName="flex flex-col gap-4 px-[18px] pb-[18px]">
        <dl className="grid grid-cols-[max-content_minmax(0,1fr)] gap-x-6 gap-y-2 text-[14px]">
          {rows.map(([label, value]) => (
            <div key={label} className="contents">
              <dt className="text-ink-3">{label}</dt>
              <dd className="m-0 break-all font-mono text-[13px] text-ink">
                {value}
              </dd>
            </div>
          ))}
        </dl>
        <div className="flex flex-wrap gap-2">
          <CopyButton
            text={session.sessionId}
            label="Copy task id"
            size="sm"
            className="h-[34px] font-semibold text-[13.5px]"
          />
          {children}
        </div>
      </CollapsibleContent>
    </Collapsible>
  )
}
