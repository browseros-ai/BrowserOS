import type { VoiceState } from '@/modules/voice/voice-types'
import type { VoiceOrbState } from './VoiceOrb'

export function chipTextFor(
  state: VoiceState,
  errorMessage: string | null,
): string {
  if (state === 'error') return errorMessage ?? 'Something went wrong'
  switch (state) {
    case 'listening':
      return 'Listening'
    case 'capturing':
      return 'Capturing'
    case 'transcribing':
      return 'Transcribing'
    case 'responding':
      return 'BrowserOS Agent is working'
    case 'barge_in_pending':
      // Do not advertise that the agent has been interrupted. The
      // interruption is still tentative; the agent is still working
      // until the transcript is confirmed real.
      return 'BrowserOS Agent is working'
    case 'closed':
    case 'idle':
      return ''
  }
}

export function showsDots(state: VoiceState): boolean {
  return state === 'capturing' || state === 'transcribing'
}

export function orbStateFor(input: {
  state: VoiceState
  isWarmingUp: boolean
}): VoiceOrbState {
  if (input.isWarmingUp) return 'idle'
  switch (input.state) {
    case 'responding':
    case 'transcribing':
      return 'speaking'
    case 'capturing':
    case 'listening':
    case 'barge_in_pending':
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
