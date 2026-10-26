import { KeyRound } from 'lucide-react'
import { type CSSProperties, useState } from 'react'
import { createPortal } from 'react-dom'
import { SIDEBAR_WIDTH } from '@/components/layout/layout.constants'
import { Button } from '@/components/motion/button/base'
import { StatefulButton } from '@/components/motion/button/stateful'
import { Card } from '@/components/motion/card'
import { Input } from '@/components/motion/input'
import type { LiveSessionCardRecord } from '@/screens/cockpit/cockpit.helpers'
import { helpRequestSite } from '@/screens/cockpit/cockpit.helpers'

interface InControlBarProps {
  session: LiveSessionCardRecord
  onHandBack: (sessionId: string, note?: string) => void
  onCancel: () => void
  isHandingBack: boolean
}

/**
 * The bar that stays on Home after Help now. The person switches to the page
 * the agent brought forward, does what it asked, then comes back here to hand
 * control back, optionally leaving the agent a note about what they did.
 */
export function InControlBar({
  session,
  onHandBack,
  onCancel,
  isHandingBack,
}: InControlBarProps) {
  const [note, setNote] = useState('')
  const help = session.helpRequest
  const app = session.agentName
  const site = help ? helpRequestSite(help) : 'the'

  // Portaled out of the page: the page enter transform would otherwise be this
  // bar's containing block and pin it below the fold until the enter ends.
  // The sidebar width variable does not reach the body, so it is set here.
  return createPortal(
    <div
      style={{ '--sidebar-width': `${SIDEBAR_WIDTH}px` } as CSSProperties}
      className="fixed inset-x-3 bottom-3 z-40 md:right-6 md:bottom-5 md:left-[calc(var(--sidebar-width)+24px)]"
    >
      <Card
        role="region"
        aria-label="You are in control"
        size="compact"
        className="mx-auto max-w-[980px] gap-4 border-border-2 bg-card px-5 py-4 shadow-pop md:grid md:grid-cols-[auto_minmax(0,1fr)_auto] md:items-center md:gap-[18px]"
      >
        <span
          aria-hidden
          className="hidden size-11 place-items-center rounded-full border border-amber/40 bg-amber-tint text-amber-ink md:grid"
        >
          <KeyRound className="size-[18px]" strokeWidth={1.75} />
        </span>
        <div className="flex min-w-0 flex-col gap-0.5">
          <p className="font-bold text-[15.5px] text-cyanotype-ink">
            You’re in control of the {site} page.
          </p>
          <p className="text-[14px] text-ink-2">
            Do what it needs there, then hand it back.
            {help?.resumeHint
              ? ` ${app} will continue with: “${help.resumeHint}”.`
              : ''}
          </p>
          <Input
            id="hand-back-note"
            aria-label={`Note for ${app} (optional)`}
            value={note}
            onChange={setNote}
            placeholder={`Anything ${app} should know? For example: signed in, code entered`}
            className="mt-2"
            classNames={{ field: 'h-10 bg-background' }}
          />
        </div>
        <div className="flex flex-wrap gap-2">
          <StatefulButton
            data-hand-back={session.sessionId}
            state={isHandingBack ? 'loading' : 'idle'}
            loadingText="Handing back"
            onClick={() => onHandBack(session.sessionId, note)}
            disabled={isHandingBack}
            className="font-semibold"
          >
            I’m done, continue
          </StatefulButton>
          <Button
            variant="ghost"
            data-cancel-control
            onClick={onCancel}
            disabled={isHandingBack}
            className="font-semibold"
          >
            Cancel
          </Button>
        </div>
      </Card>
    </div>,
    document.body,
  )
}
