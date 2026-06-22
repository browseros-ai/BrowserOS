import type { PersonaState } from '@/components/ai-elements/persona'
import type { VoiceState } from '@/modules/voice/voice-types'

export function chipTextFor(
  state: VoiceState,
  isBargingIn: boolean,
  errorMessage: string | null,
): string {
  if (state === 'error') return errorMessage ?? 'Something went wrong'
  if (state === 'responding' && isBargingIn) return 'Listening over agent'
  switch (state) {
    case 'listening':
      return 'Listening'
    case 'capturing':
      return 'Capturing'
    case 'transcribing':
      return 'Transcribing'
    case 'responding':
      return 'Speaking'
    case 'closed':
    case 'idle':
      return ''
  }
}

export function showsDots(state: VoiceState): boolean {
  return state === 'capturing' || state === 'transcribing'
}

export function personaStateFor(input: {
  state: VoiceState
  isWarmingUp: boolean
}): PersonaState {
  if (input.isWarmingUp) return 'idle'
  switch (input.state) {
    case 'responding':
      return 'speaking'
    case 'transcribing':
      return 'thinking'
    case 'capturing':
    case 'listening':
      return 'listening'
    default:
      return 'idle'
  }
}

export function aggregateLevel(levels: number[]): number {
  if (levels.length === 0) return 0
  let sum = 0
  for (const v of levels) sum += v
  return sum / levels.length
}

export function haloAmplitudeFor(input: {
  state: VoiceState
  audioLevels: number[]
}): number {
  if (input.state === 'capturing')
    return aggregateLevel(input.audioLevels) / 100
  return 0
}
