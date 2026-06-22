import { createStore } from '@xstate/store'
import type { VoiceContext } from './voice-types'

export const INITIAL_CONTEXT: VoiceContext = {
  state: 'idle',
  audioLevels: [0, 0, 0, 0, 0],
  errorMessage: null,
  isBargingIn: false,
  isWarmingUp: false,
}

export function createVoiceLoopStore() {
  return createStore({
    context: INITIAL_CONTEXT,
    emits: {
      runTranscribe: (_p: { blob: Blob }) => undefined,
      sendChatMessage: (_p: { text: string }) => undefined,
      cancelChatStream: () => undefined,
      releaseCapture: () => undefined,
      markLastAssistantInterrupted: () => undefined,
    },
    on: {
      OPEN: (ctx, _e: object) => ({
        ...ctx,
        state: 'listening' as const,
        errorMessage: null,
        isBargingIn: false,
        isWarmingUp: true,
      }),

      WARM_UP_DONE: (ctx, _e: object) => ({
        ...ctx,
        isWarmingUp: false,
      }),

      SPEECH_START: (ctx, _e: object) => {
        if (ctx.state === 'listening') {
          return { ...ctx, state: 'capturing' as const }
        }
        if (ctx.state === 'responding') {
          return { ...ctx, isBargingIn: true }
        }
        return ctx
      },

      SPEECH_END: (ctx, event: { blob: Blob }, enqueue) => {
        if (ctx.state === 'capturing') {
          enqueue.emit.runTranscribe({ blob: event.blob })
          return { ...ctx, state: 'transcribing' as const }
        }
        if (ctx.state === 'responding' && ctx.isBargingIn) {
          enqueue.emit.cancelChatStream()
          enqueue.emit.markLastAssistantInterrupted()
          enqueue.emit.runTranscribe({ blob: event.blob })
          return { ...ctx, state: 'transcribing' as const, isBargingIn: false }
        }
        return ctx
      },

      TRANSCRIBE_OK: (ctx, event: { text: string }, enqueue) => {
        if (ctx.state !== 'transcribing') return ctx
        enqueue.emit.sendChatMessage({ text: event.text })
        return { ...ctx, state: 'responding' as const }
      },

      TRANSCRIBE_FAIL: (ctx, event: { message: string }) => {
        if (ctx.state !== 'transcribing') return ctx
        return { ...ctx, state: 'error' as const, errorMessage: event.message }
      },

      CHAT_STREAMING_ENDED: (ctx, _e: object) => {
        if (ctx.state !== 'responding') return ctx
        return { ...ctx, state: 'listening' as const, isBargingIn: false }
      },

      STOP_AGENT: (ctx, _e: object, enqueue) => {
        if (ctx.state !== 'responding') return ctx
        enqueue.emit.cancelChatStream()
        enqueue.emit.markLastAssistantInterrupted()
        return { ...ctx, state: 'listening' as const, isBargingIn: false }
      },

      CLOSE: (_ctx, _e: object, enqueue) => {
        enqueue.emit.releaseCapture()
        return { ...INITIAL_CONTEXT, state: 'closed' as const }
      },

      ERROR: (ctx, event: { message: string }) => ({
        ...ctx,
        state: 'error' as const,
        errorMessage: event.message,
      }),

      RETRY: (ctx, _e: object) => {
        if (ctx.state !== 'error') return ctx
        return { ...ctx, state: 'listening' as const, errorMessage: null }
      },

      AUDIO_LEVELS: (ctx, event: { levels: number[] }) => ({
        ...ctx,
        audioLevels: event.levels,
      }),
    },
  })
}

export type VoiceLoopStore = ReturnType<typeof createVoiceLoopStore>
