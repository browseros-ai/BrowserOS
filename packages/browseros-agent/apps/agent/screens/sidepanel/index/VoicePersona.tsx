import { cn } from '@/lib/utils'

export type VoicePersonaState =
  | 'idle'
  | 'listening'
  | 'thinking'
  | 'speaking'
  | 'asleep'

interface VoicePersonaProps {
  state: VoicePersonaState
  amplitude: number
  className?: string
}

export function VoicePersona({
  state,
  amplitude,
  className,
}: VoicePersonaProps) {
  const scale = 1 + Math.min(0.5, Math.max(0, amplitude) * 0.6)
  const animation = animationFor(state)
  return (
    <div className={cn('relative grid place-items-center', className)}>
      <div
        className={cn(
          'size-12 rounded-full shadow-lg ring-2 ring-white/30 transition-transform duration-150',
          animation,
        )}
        style={{
          transform: `scale(${scale.toFixed(3)})`,
          background:
            'radial-gradient(circle at 30% 30%, var(--accent-orange-bright), var(--accent-orange) 60%, color-mix(in oklab, var(--accent-orange) 70%, black) 100%)',
        }}
        aria-hidden
      />
    </div>
  )
}

function animationFor(state: VoicePersonaState): string {
  switch (state) {
    case 'listening':
      return 'animate-[voice-persona-breath_3.5s_ease-in-out_infinite]'
    case 'thinking':
      return 'animate-spin opacity-80'
    case 'speaking':
      return 'animate-[voice-persona-pulse_900ms_ease-in-out_infinite]'
    case 'asleep':
      return 'opacity-50'
    default:
      return 'opacity-70'
  }
}
