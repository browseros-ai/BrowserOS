import type { VoiceLoopStore } from './voice-loop.store'

export type VoiceState =
  | 'idle'
  | 'listening'
  | 'capturing'
  | 'transcribing'
  | 'responding'
  | 'barge_in_pending'
  | 'error'
  | 'closed'

// `origin` records whether the in-flight transcription came from a
// normal listening turn or from a tentative barge-in. The
// TRANSCRIBE_OK / TRANSCRIBE_DROPPED handlers branch on this to
// decide whether the in-flight agent reply should be cancelled.
export type TurnOrigin = 'normal' | 'barge_in_pending'

export interface VoiceContext {
  state: VoiceState
  audioLevels: number[]
  errorMessage: string | null
  isWarmingUp: boolean
  origin: TurnOrigin
  // Set when the chat session finishes streaming while the loop is
  // mid-barge-in (pending or transcribing). Consumed by the
  // TRANSCRIBE_DROPPED / TRANSCRIBE_FAIL pending branches to unwind
  // to listening instead of a stuck `responding`.
  chatStreamEndedWhilePending: boolean
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
