import { Briefcase, Check, Loader2, Scale, SmilePlus, Zap } from 'lucide-react'
import { useState } from 'react'
import { Button } from '@/components/ui/button'
import {
  ONBOARDING_SOUL_SELECTED_EVENT,
  ONBOARDING_STEP_COMPLETED_EVENT,
} from '@/lib/constants/analyticsEvents'
import { track } from '@/lib/metrics/track'
import { type SoulPresetId, soulPresets } from '@/lib/onboarding/soulPresets'
import { useRpcClient } from '@/lib/rpc/RpcClientProvider'
import { sentry } from '@/lib/sentry/sentry'
import { cn } from '@/lib/utils'
import { type StepDirection, StepTransition } from './StepTransition'

interface StepSoulProps {
  direction: StepDirection
  onContinue: () => void
}

const presetIcons: Record<SoulPresetId, typeof Scale> = {
  balanced: Scale,
  professional: Briefcase,
  friendly: SmilePlus,
  minimal: Zap,
}

export const StepSoul = ({ direction, onContinue }: StepSoulProps) => {
  const [selected, setSelected] = useState<SoulPresetId>('balanced')
  const [isSaving, setIsSaving] = useState(false)
  const rpcClient = useRpcClient()

  const handleContinue = async () => {
    const preset = soulPresets.find((p) => p.id === selected)
    if (!preset) return

    setIsSaving(true)
    try {
      await rpcClient.soul.$put({
        json: { content: preset.content },
      })
    } catch (e) {
      sentry.captureException(e, {
        extra: { message: 'Failed to write soul during onboarding' },
      })
    } finally {
      setIsSaving(false)
    }

    track(ONBOARDING_SOUL_SELECTED_EVENT, { preset: selected })
    track(ONBOARDING_STEP_COMPLETED_EVENT, {
      step: 2,
      step_name: 'soul',
    })
    onContinue()
  }

  return (
    <StepTransition direction={direction}>
      <div className="flex h-full flex-col items-center justify-center">
        <div className="w-full max-w-md space-y-6">
          <div className="space-y-2 text-center">
            <h2 className="font-bold text-3xl tracking-tight">
              Choose your Sup's personality
            </h2>
            <p className="text-base text-muted-foreground">
              This sets the starting tone — you can always evolve it later
            </p>
          </div>

          <div className="grid grid-cols-2 gap-3">
            {soulPresets.map((preset) => {
              const Icon = presetIcons[preset.id]
              const isSelected = selected === preset.id

              return (
                <button
                  key={preset.id}
                  type="button"
                  onClick={() => setSelected(preset.id)}
                  className={cn(
                    'relative flex flex-col items-center gap-3 rounded-xl border-2 p-5 text-center transition-all',
                    isSelected
                      ? 'border-[var(--accent-orange)] bg-[var(--accent-orange)]/5'
                      : 'border-border bg-card hover:border-[var(--accent-orange)]/50',
                  )}
                >
                  {isSelected && (
                    <div className="absolute top-2.5 right-2.5 flex size-5 items-center justify-center rounded-full bg-[var(--accent-orange)]">
                      <Check className="size-3 text-white" />
                    </div>
                  )}
                  <div
                    className={cn(
                      'flex size-10 items-center justify-center rounded-full',
                      isSelected
                        ? 'bg-[var(--accent-orange)]/10 text-[var(--accent-orange)]'
                        : 'bg-muted text-muted-foreground',
                    )}
                  >
                    <Icon className="size-5" />
                  </div>
                  <div>
                    <div className="font-semibold text-sm">{preset.name}</div>
                    <div className="mt-1 text-muted-foreground text-xs">
                      {preset.description}
                    </div>
                  </div>
                </button>
              )
            })}
          </div>

          <Button
            onClick={handleContinue}
            disabled={isSaving}
            className="w-full bg-[var(--accent-orange)] text-white hover:bg-[var(--accent-orange)]/90"
          >
            {isSaving ? (
              <Loader2 className="size-4 animate-spin" />
            ) : (
              'Continue'
            )}
          </Button>
        </div>
      </div>
    </StepTransition>
  )
}
