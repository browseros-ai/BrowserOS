import { createStore } from '@xstate/store'
import type { DropReason } from './transcript-sanitizer'
import type { VoiceContext } from './voice-types'

export const INITIAL_CONTEXT: VoiceContext = {
  state: 'idle',
  audioLevels: [0, 0, 0, 0, 0],
  errorMessage: null,
  isWarmingUp: false,
  origin: 'normal',
  chatStreamEndedWhilePending: false,
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
        isWarmingUp: true,
        origin: 'normal' as const,
        chatStreamEndedWhilePending: false,
      }),

      WARM_UP_DONE: (ctx, _e: object) => ({
        ...ctx,
        isWarmingUp: false,
      }),

      SPEECH_START: (ctx, _e: object) => {
        if (ctx.state === 'listening') {
          return { ...ctx, state: 'capturing' as const }
        }
        return ctx
      },

      // Tentative barge-in. Speech-start was detected while the agent
      // is responding. We start recording but do NOT cancel the agent
      // here. Cancellation only happens after we have a sanitized
      // transcript that turns out to be real speech.
      BARGE_IN_TENTATIVE: (ctx, _e: object) => {
        if (ctx.state !== 'responding') return ctx
        return {
          ...ctx,
          state: 'barge_in_pending' as const,
          origin: 'barge_in_pending' as const,
          chatStreamEndedWhilePending: false,
        }
      },

      // The VAD reported speech-start but the segment never met the
      // minimum duration. Unwind to the previous state. We also use
      // chatStreamEndedWhilePending here so a sub-minimum barge-in
      // blip that happens after the agent already finished streaming
      // ends up in listening instead of a stuck responding.
      SPEECH_ABORTED: (ctx, _e: object) => {
        if (ctx.state === 'capturing') {
          return { ...ctx, state: 'listening' as const }
        }
        if (ctx.state === 'barge_in_pending') {
          if (ctx.chatStreamEndedWhilePending) {
            return {
              ...ctx,
              state: 'listening' as const,
              origin: 'normal' as const,
              chatStreamEndedWhilePending: false,
            }
          }
          return {
            ...ctx,
            state: 'responding' as const,
            origin: 'normal' as const,
          }
        }
        return ctx
      },

      SPEECH_END: (ctx, event: { blob: Blob }, enqueue) => {
        if (ctx.state === 'capturing') {
          enqueue.emit.runTranscribe({ blob: event.blob })
          return {
            ...ctx,
            state: 'transcribing' as const,
            origin: 'normal' as const,
          }
        }
        if (ctx.state === 'barge_in_pending') {
          enqueue.emit.runTranscribe({ blob: event.blob })
          return {
            ...ctx,
            state: 'transcribing' as const,
            origin: 'barge_in_pending' as const,
          }
        }
        return ctx
      },

      TRANSCRIBE_OK: (ctx, event: { text: string }, enqueue) => {
        if (ctx.state !== 'transcribing') return ctx
        if (ctx.origin === 'barge_in_pending') {
          enqueue.emit.cancelChatStream()
          enqueue.emit.markLastAssistantInterrupted()
        }
        enqueue.emit.sendChatMessage({ text: event.text })
        return {
          ...ctx,
          state: 'responding' as const,
          origin: 'normal' as const,
          chatStreamEndedWhilePending: false,
        }
      },

      TRANSCRIBE_DROPPED: (ctx, _event: { reason: DropReason }) => {
        if (ctx.state !== 'transcribing') return ctx
        if (ctx.origin === 'barge_in_pending') {
          // The agent finished streaming while we were in
          // pending/transcribing. Returning to responding would deadlock
          // because the streaming->idle edge has already passed; go
          // straight to listening instead.
          if (ctx.chatStreamEndedWhilePending) {
            return {
              ...ctx,
              state: 'listening' as const,
              origin: 'normal' as const,
              chatStreamEndedWhilePending: false,
            }
          }
          // Tentative barge-in turned out to be noise. Resume
          // responding as if nothing happened; the agent is still
          // working.
          return {
            ...ctx,
            state: 'responding' as const,
            origin: 'normal' as const,
          }
        }
        return {
          ...ctx,
          state: 'listening' as const,
          origin: 'normal' as const,
        }
      },

      TRANSCRIBE_FAIL: (ctx, event: { message: string }) => {
        if (ctx.state !== 'transcribing') return ctx
        // A failed transcribe during pending barge-in must not surface
        // an error chip and must not interrupt the agent. Treat it as
        // a quiet "noise" drop. If the chat stream already ended,
        // unwind to listening to avoid the deadlocked-responding bug.
        if (ctx.origin === 'barge_in_pending') {
          if (ctx.chatStreamEndedWhilePending) {
            return {
              ...ctx,
              state: 'listening' as const,
              origin: 'normal' as const,
              chatStreamEndedWhilePending: false,
            }
          }
          return {
            ...ctx,
            state: 'responding' as const,
            origin: 'normal' as const,
          }
        }
        return { ...ctx, state: 'error' as const, errorMessage: event.message }
      },

      CHAT_STREAMING_ENDED: (ctx, _e: object) => {
        if (ctx.state === 'responding') {
          return { ...ctx, state: 'listening' as const }
        }
        // We caught the streaming->idle edge while the loop was busy
        // handling a tentative barge-in. Stash the fact so the
        // eventual TRANSCRIBE_DROPPED/FAIL knows to unwind to listening
        // instead of returning to a state the chat session has
        // already left.
        if (
          ctx.state === 'barge_in_pending' ||
          (ctx.state === 'transcribing' && ctx.origin === 'barge_in_pending')
        ) {
          return { ...ctx, chatStreamEndedWhilePending: true }
        }
        return ctx
      },

      STOP_AGENT: (ctx, _e: object, enqueue) => {
        if (ctx.state !== 'responding' && ctx.state !== 'barge_in_pending')
          return ctx
        enqueue.emit.cancelChatStream()
        enqueue.emit.markLastAssistantInterrupted()
        return {
          ...ctx,
          state: 'listening' as const,
          origin: 'normal' as const,
          chatStreamEndedWhilePending: false,
        }
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

      AUDIO_LEVELS: (ctx, event: { levels: number[] }) => ({
        ...ctx,
        audioLevels: event.levels,
      }),
    },
  })
}

export type VoiceLoopStore = ReturnType<typeof createVoiceLoopStore>
