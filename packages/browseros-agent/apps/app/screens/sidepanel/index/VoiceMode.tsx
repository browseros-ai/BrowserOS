import { useSelector } from '@xstate/store/react'
import { X } from 'lucide-react'
import { memo } from 'react'
import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'
import type { VoiceLoopApi } from '@/modules/voice/voice-types'
import { VoiceOrb } from './VoiceOrb'
import {
  chipTextFor,
  haloAmplitudeFor,
  orbStateFor,
  showsDots,
} from './voice-mode.helpers'

interface VoiceModeProps {
  api: VoiceLoopApi
}

export const VoiceMode = memo(function VoiceMode({ api }: VoiceModeProps) {
  const state = useSelector(api.store, (s) => s.context.state)
  const audioLevels = useSelector(api.store, (s) => s.context.audioLevels)
  const errorMessage = useSelector(api.store, (s) => s.context.errorMessage)
  const isWarmingUp = useSelector(api.store, (s) => s.context.isWarmingUp)

  const chip = chipTextFor(state, errorMessage)
  const orbState = orbStateFor({ state, isWarmingUp })
  const halo = haloAmplitudeFor({ state, audioLevels })
  const dots = showsDots(state)
  const stopEnabled = state === 'responding'
  const isError = state === 'error'
  const isResponding = state === 'responding'

  return (
    <section
      aria-label="Voice mode"
      className="absolute inset-0 flex flex-col items-center justify-between rounded-lg bg-gradient-to-b from-accent/15 via-accent/5 to-accent/20 p-3"
    >
      <header className="flex w-full items-center justify-between">
        <button
          type="button"
          onClick={api.close}
          aria-label="Close voice mode"
          className="rounded-md p-1 text-muted-foreground hover:bg-accent/30 hover:text-foreground"
        >
          <X className="h-3.5 w-3.5" />
        </button>
        <span className="font-medium text-muted-foreground text-xs">
          Voice mode
        </span>
        <span className="w-6" aria-hidden />
      </header>

      <div className="relative flex flex-1 flex-col items-center justify-center">
        <div
          aria-hidden
          className={cn(
            'absolute rounded-full blur-xl transition-[width,height,opacity] duration-150',
            isResponding &&
              'animate-[voice-halo-pulse_1100ms_ease-in-out_infinite]',
          )}
          style={{
            width: `${96 + halo * 64}px`,
            height: `${96 + halo * 64}px`,
            opacity: isResponding ? 0.55 : 0.25 + halo * 0.45,
            background:
              'color-mix(in oklab, var(--accent-orange) 60%, transparent)',
          }}
        />
        <VoiceOrb state={orbState} size={120} className="relative" />
        <div
          className={cn(
            'mt-2 flex items-center gap-1.5 font-medium text-muted-foreground text-xs',
            isError && 'text-destructive',
          )}
        >
          <span>{chip}</span>
          {dots && <LoadingDots />}
        </div>
      </div>

      <footer className="flex w-full items-center justify-center gap-2">
        <Button
          type="button"
          variant="outline"
          size="sm"
          onClick={api.close}
          className="min-w-[7rem]"
        >
          Close voice
        </Button>
        {isError ? (
          <Button
            type="button"
            variant="default"
            size="sm"
            onClick={api.retry}
            className="min-w-[7rem]"
          >
            Retry
          </Button>
        ) : (
          <Button
            type="button"
            variant="destructive"
            size="sm"
            onClick={api.stopAgentActivity}
            disabled={!stopEnabled}
            className="min-w-[7rem]"
          >
            Stop agent
          </Button>
        )}
      </footer>
    </section>
  )
})

function LoadingDots() {
  return (
    <span className="inline-flex items-end gap-0.5" aria-hidden>
      <Dot delayMs={0} />
      <Dot delayMs={200} />
      <Dot delayMs={400} />
    </span>
  )
}

function Dot({ delayMs }: { delayMs: number }) {
  return (
    <span
      className="size-1 animate-pulse rounded-full bg-current"
      style={{ animationDelay: `${delayMs}ms`, animationDuration: '1.2s' }}
    />
  )
}
