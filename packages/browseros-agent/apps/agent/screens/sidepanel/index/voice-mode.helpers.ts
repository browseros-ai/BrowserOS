import type { VoiceLoopApi, VoiceState } from '@/modules/voice/voice-types'
import type { VoicePersonaState } from './VoicePersona'

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

export function personaStateFor(
  api: Pick<VoiceLoopApi, 'state' | 'isWarmingUp'>,
): VoicePersonaState {
  if (api.isWarmingUp) return 'asleep'
  switch (api.state) {
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

export function synthesizedSpeakingEnvelope(nowMs: number): number {
  const phase = (nowMs / 1000) * Math.PI * 2 * 1.5
  return 0.45 + 0.15 * Math.sin(phase)
}

export function haloAmplitudeFor(
  api: Pick<VoiceLoopApi, 'state' | 'audioLevels'>,
  nowMs: number,
): number {
  if (api.state === 'capturing') return aggregateLevel(api.audioLevels) / 100
  if (api.state === 'responding') return synthesizedSpeakingEnvelope(nowMs)
  return 0
}
