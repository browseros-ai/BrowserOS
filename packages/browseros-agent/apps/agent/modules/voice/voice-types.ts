import type { VoiceLoopStore } from './voice-loop.store'

export type VoiceState =
  | 'idle'
  | 'listening'
  | 'capturing'
  | 'transcribing'
  | 'responding'
  | 'error'
  | 'closed'

export interface VoiceContext {
  state: VoiceState
  audioLevels: number[]
  errorMessage: string | null
  isBargingIn: boolean
  isWarmingUp: boolean
}

// The api exposes the underlying store and a small set of stable
// callbacks; consumers subscribe to the slices they actually need
// via `useSelector(api.store, s => s.context.x)`. This keeps the
// component that calls `useVoiceLoop` from re-rendering at the
// store's dispatch rate (which is what previously cascaded the
// renders into ChatFooter and the Persona).
export interface VoiceLoopApi {
  readonly store: VoiceLoopStore
  readonly interruptedMessageIds: ReadonlySet<string>
  open(): Promise<void>
  close(): void
  stopAgentActivity(): void
  retry(): void
}
