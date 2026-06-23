import { Check, Globe, StopCircle, User } from 'lucide-react'
import { useState } from 'react'
import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'
import type { PendingHandoff } from '@/modules/api/run.hooks'

interface HandoffBannerProps {
  handoff: PendingHandoff
  onContinue: () => void
  onCancel: () => void
}

/**
 * Full overlay rendered on top of the browser viewport when the run
 * needs the user. Amber top strip with the "Your turn" instruction +
 * dimmed page beneath + a centered card standing in for the site's
 * own challenge (CAPTCHA / 2FA). The user clears the challenge
 * natively in the live tab; this card is just a visual proxy in the
 * cockpit.
 */
export function HandoffBanner({
  handoff,
  onContinue,
  onCancel,
}: HandoffBannerProps) {
  const [checked, setChecked] = useState(false)
  return (
    <div className="absolute inset-0 z-40 flex flex-col">
      <div className="flex items-center gap-3 bg-gradient-to-r from-[#B47814] to-[#C98A1B] px-5 py-3 text-white shadow-lg">
        <span className="flex size-7 shrink-0 items-center justify-center rounded-lg bg-white/20">
          <User className="size-4" />
        </span>
        <div className="min-w-0 flex-1">
          <div className="font-extrabold text-sm">
            Your turn. Finish this verification, then hit Continue.
          </div>
          <div className="text-white/90 text-xs">{handoff.note}</div>
        </div>
      </div>
      <div className="flex flex-1 items-center justify-center bg-[rgba(20,24,30,0.34)] p-6 backdrop-blur-sm">
        <div className="w-[384px] max-w-[92%] overflow-hidden rounded-2xl bg-card shadow-2xl">
          <header className="border-border border-b px-5 py-4">
            <div className="mb-1 flex items-center gap-1.5 text-ink-3 text-xs">
              <Globe className="size-3" />
              <span className="font-mono">{handoff.domain}</span>
            </div>
            <h2 className="font-bold text-[15px] text-ink">{handoff.title}</h2>
            <p className="mt-1 text-ink-2 text-xs leading-snug">
              {handoff.detail}
            </p>
          </header>
          <div className="p-5">
            <button
              type="button"
              onClick={() => setChecked((c) => !c)}
              className="flex w-full items-center gap-3 rounded-xl border border-border-2 bg-bg-sunken p-3.5"
            >
              <span
                className={cn(
                  'flex size-6 shrink-0 items-center justify-center rounded-md border-2 transition-colors',
                  checked
                    ? 'border-green bg-green text-card'
                    : 'border-border-strong bg-card',
                )}
              >
                {checked && <Check className="size-3.5" />}
              </span>
              <span className="font-semibold text-ink text-sm">
                I'm not a robot
              </span>
              <span className="flex-1" />
              <span className="text-right text-[10px] text-ink-4 leading-tight">
                Security
                <br />
                check
              </span>
            </button>
            <div className="mt-4 flex gap-2">
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
      </div>
    </div>
  )
}
