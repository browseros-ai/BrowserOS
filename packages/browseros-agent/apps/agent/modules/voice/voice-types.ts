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
}

export interface VoiceLoopApi {
  readonly state: VoiceState
  readonly audioLevels: number[]
  readonly errorMessage: string | null
  readonly isBargingIn: boolean
  readonly isWarmingUp: boolean
  readonly interruptedMessageIds: ReadonlySet<string>
  open(): Promise<void>
  close(): void
  stopAgentActivity(): void
  retry(): void
}
