import { useState } from 'react'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import type { LiveSessionCardRecord } from '@/screens/cockpit/cockpit.helpers'
import { siteOf } from '@/screens/cockpit/cockpit.helpers'
import { needsYouTone } from './needsYouTone'

interface InControlBarProps {
  session: LiveSessionCardRecord
  onHandBack: (sessionId: string, note?: string) => void
  onCancel: () => void
  isHandingBack: boolean
}

/**
 * The bar that persists in the cockpit tab after Take over. The human switches
 * to the agent's foregrounded tab, clears the block there, then returns here to
 * hand control back, optionally leaving the agent a note about what they did.
 */
export function InControlBar({
  session,
  onHandBack,
  onCancel,
  isHandingBack,
}: InControlBarProps) {
  const [note, setNote] = useState('')
  const help = session.helpRequest
  const site = help?.url ? siteOf(help.url) : undefined

  return (
    <div className="fixed inset-x-0 bottom-0 z-40 border-amber/40 border-t bg-amber-tint/95 px-4 py-3 backdrop-blur">
      <div className="mx-auto flex max-w-[1040px] flex-col gap-2">
        <div className="flex flex-col gap-0.5">
          <p className="font-semibold text-[14px] text-ink">
            You are in control of {session.label}
            {site ? ` on ${site}` : ''}.
          </p>
          {help?.resumeHint && (
            <p className="text-[13px] text-ink-2">
              When done, the agent resumes at: {help.resumeHint}
            </p>
          )}
        </div>
        <div className="flex flex-wrap items-end gap-2">
          <div className="flex min-w-0 flex-1 flex-col gap-1">
            <Label
              htmlFor="hand-back-note"
              className="font-mono text-[10.5px] text-ink-3 uppercase tracking-[0.08em]"
            >
              Note for the agent (optional)
            </Label>
            <Input
              id="hand-back-note"
              value={note}
              onChange={(event) => setNote(event.target.value)}
              placeholder="e.g. signed in, 2FA done"
              className="bg-card"
            />
          </div>
          <Button
            type="button"
            size="default"
            data-hand-back={session.sessionId}
            onClick={() => onHandBack(session.sessionId, note)}
            disabled={isHandingBack}
            className={needsYouTone.action}
          >
            {isHandingBack ? 'Handing back…' : 'Hand back to the agent'}
          </Button>
          <Button
            type="button"
            size="default"
            variant="ghost"
            data-cancel-control
            onClick={onCancel}
            disabled={isHandingBack}
          >
            Cancel
          </Button>
        </div>
      </div>
    </div>
  )
}
