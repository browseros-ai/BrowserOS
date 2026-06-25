import type { UIMessage } from 'ai'
import type { RefObject } from 'react'
import { useEffect, useRef, useState } from 'react'
import {
  SIDEPANEL_VOICE_MODE_BARGE_IN_EVENT,
  SIDEPANEL_VOICE_MODE_CLOSED_EVENT,
  SIDEPANEL_VOICE_MODE_OPENED_EVENT,
  SIDEPANEL_VOICE_MODE_STOP_AGENT_EVENT,
  SIDEPANEL_VOICE_MODE_TRANSCRIBE_FAILED_EVENT,
  SIDEPANEL_VOICE_MODE_TURN_CAPTURED_EVENT,
} from '@/lib/constants/analyticsEvents'
import { track } from '@/lib/metrics/track'
import { transcribeAudio } from '@/lib/voice/transcribe-audio'
import {
  type AudioCaptureHandle,
  describeCaptureError,
  openAudioCapture,
} from './audio-capture'
import {
  type AudioLevelMonitor,
  createAudioLevelMonitor,
} from './audio-level-monitor'
import { sanitize } from './transcript-sanitizer'
import { createVad, type VadHandle } from './vad'
import { voiceDebug } from './voice-debug'
import { createVoiceLoopStore } from './voice-loop.store'
import type { VoiceLoopApi } from './voice-types'

const WARM_UP_MS = 800
const WAVEFORM_BAND_COUNT = 5
const STATUS_POLL_MS = 200

export interface ChatSessionLike {
  sendMessage: (params: { text: string }) => void
  stop: () => void
  status: string
  messages: UIMessage[]
}

export interface UseVoiceLoopOptions {
  chatSessionRef: RefObject<ChatSessionLike | null>
}

export function useVoiceLoop(opts: UseVoiceLoopOptions): VoiceLoopApi {
  const [store] = useState(() => createVoiceLoopStore())

  const captureRef = useRef<AudioCaptureHandle | null>(null)
  const monitorRef = useRef<AudioLevelMonitor | null>(null)
  const vadRef = useRef<VadHandle | null>(null)
  const recorderRef = useRef<MediaRecorder | null>(null)
  const transcribeAbortRef = useRef<AbortController | null>(null)
  const warmUpTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const interruptedIdsRef = useRef<Set<string>>(new Set())
  const stateSubRef = useRef<{ unsubscribe: () => void } | null>(null)

  // Poll the chat session's status from the ref instead of subscribing
  // to it via React deps. The chat session updates dozens of times a
  // second while a reply is streaming; consuming `status` as a render
  // dep used to re-run this whole hook every token. The poll is a
  // 5 Hz timer that does nothing besides compare two strings.
  useEffect(() => {
    let prev = opts.chatSessionRef.current?.status
    const id = setInterval(() => {
      const next = opts.chatSessionRef.current?.status
      if (prev === 'streaming' && next !== 'streaming') {
        voiceDebug('chat streaming ended', next)
        store.send({ type: 'CHAT_STREAMING_ENDED' })
      }
      prev = next
    }, STATUS_POLL_MS)
    return () => clearInterval(id)
  }, [opts.chatSessionRef, store])

  // biome-ignore lint/correctness/useExhaustiveDependencies: releaseResources is a stable local helper; depending on it would re-subscribe emits on every render
  useEffect(() => {
    const subs = [
      store.on('runTranscribe', async ({ blob }) => {
        transcribeAbortRef.current?.abort()
        const ac = new AbortController()
        transcribeAbortRef.current = ac
        try {
          voiceDebug('transcribe request', { bytes: blob.size })
          const result = await transcribeAudio(blob)
          if (ac.signal.aborted) return
          voiceDebug('transcribe response', {
            chars: result.text.length,
            avgLogprob: result.avgLogprob,
          })
          const verdict = sanitize(result.text, {
            avgLogprob: result.avgLogprob,
          })
          if (verdict.action === 'drop') {
            voiceDebug('sanitize drop', verdict.reason)
            store.send({ type: 'TRANSCRIBE_DROPPED', reason: verdict.reason })
            return
          }
          voiceDebug('sanitize send', { chars: verdict.text.length })
          // Only count a barge-in once the transcript clears the
          // sanitizer; tentative triggers (chair scrape, chime, brief
          // cough) used to inflate this metric.
          if (store.getSnapshot().context.origin === 'barge_in_pending') {
            track(SIDEPANEL_VOICE_MODE_BARGE_IN_EVENT)
          }
          track(SIDEPANEL_VOICE_MODE_TURN_CAPTURED_EVENT, {
            chars: verdict.text.length,
          })
          store.send({ type: 'TRANSCRIBE_OK', text: verdict.text })
        } catch (err) {
          if (ac.signal.aborted) return
          const message =
            err instanceof Error ? err.message : 'Transcription failed'
          voiceDebug('transcribe error', message)
          track(SIDEPANEL_VOICE_MODE_TRANSCRIBE_FAILED_EVENT, {
            reason: 'error',
          })
          store.send({ type: 'TRANSCRIBE_FAIL', message })
        }
      }),
      store.on('sendChatMessage', ({ text }) => {
        voiceDebug('send chat message', { chars: text.length })
        opts.chatSessionRef.current?.sendMessage({ text })
      }),
      store.on('cancelChatStream', () => {
        voiceDebug('cancel chat stream')
        opts.chatSessionRef.current?.stop()
      }),
      store.on('markLastAssistantInterrupted', () => {
        const messages = opts.chatSessionRef.current?.messages
        if (!messages) return
        const last = lastAssistantId(messages)
        if (last && !interruptedIdsRef.current.has(last)) {
          interruptedIdsRef.current.add(last)
        }
      }),
      store.on('releaseCapture', () => {
        releaseResources()
      }),
    ]
    return () => {
      for (const s of subs) s.unsubscribe()
    }
  }, [store, opts.chatSessionRef])

  const releaseResources = () => {
    transcribeAbortRef.current?.abort()
    transcribeAbortRef.current = null
    if (recorderRef.current && recorderRef.current.state !== 'inactive') {
      try {
        recorderRef.current.stop()
      } catch {
        // ignore
      }
    }
    recorderRef.current = null
    stateSubRef.current?.unsubscribe()
    stateSubRef.current = null
    vadRef.current?.stop()
    vadRef.current = null
    monitorRef.current?.stop()
    monitorRef.current = null
    captureRef.current?.close()
    captureRef.current = null
    if (warmUpTimerRef.current !== null) {
      clearTimeout(warmUpTimerRef.current)
      warmUpTimerRef.current = null
    }
  }

  // biome-ignore lint/correctness/useExhaustiveDependencies: cleanup runs only on unmount; closing over latest refs is intentional
  useEffect(() => {
    return () => {
      releaseResources()
    }
  }, [])

  const open = async (): Promise<void> => {
    if (captureRef.current) return
    try {
      const capture = await openAudioCapture()
      captureRef.current = capture

      const monitor = createAudioLevelMonitor({
        bandCount: WAVEFORM_BAND_COUNT,
      })
      // Throttle the AUDIO_LEVELS store dispatch to ~12 Hz. The
      // monitor itself ticks at the display's rAF rate (60-120 Hz).
      // 12 Hz is fast enough for the persona halo to track the voice
      // smoothly and slow enough that React reconciliation does not
      // compete with Rive's canvas loop on the main thread.
      let lastLevelsAt = 0
      monitor.subscribe((sample) => {
        const now = performance.now()
        if (now - lastLevelsAt < 80) return
        lastLevelsAt = now
        store.send({ type: 'AUDIO_LEVELS', levels: sample.levels })
      })
      monitor.start(capture.analyser)
      monitorRef.current = monitor

      const mimeType = MediaRecorder.isTypeSupported('audio/webm;codecs=opus')
        ? 'audio/webm;codecs=opus'
        : 'audio/webm'

      const startTurnRecorder = () => {
        const chunks: Blob[] = []
        const rec = new MediaRecorder(capture.stream, { mimeType })
        rec.ondataavailable = (e) => {
          if (e.data.size > 0) chunks.push(e.data)
        }
        rec.onstop = () => {
          if (chunks.length === 0) return
          const blob = new Blob(chunks, { type: mimeType })
          store.send({ type: 'SPEECH_END', blob })
        }
        rec.start()
        recorderRef.current = rec
      }

      const vad = await createVad(capture, monitor, {
        onSpeechStart: () => {
          const current = store.getSnapshot().context.state
          if (current === 'responding') {
            // Tentative barge-in: start recording but keep the agent
            // running. The store will only cancel after the transcript
            // is confirmed real by the sanitizer; the BARGE_IN_EVENT
            // analytic fires there too, not here.
            voiceDebug('barge-in tentative')
            startTurnRecorder()
            store.send({ type: 'BARGE_IN_TENTATIVE' })
            return
          }
          voiceDebug('speech start')
          startTurnRecorder()
          store.send({ type: 'SPEECH_START' })
        },
        onSpeechEnd: () => {
          // The recorder's onstop handler dispatches SPEECH_END with
          // the freshly framed WebM blob; we just stop it here.
          voiceDebug('speech end')
          const rec = recorderRef.current
          recorderRef.current = null
          if (rec && rec.state !== 'inactive') {
            rec.stop()
          }
        },
        onSpeechAbort: () => {
          // Energy VAD speculatively fired speech-start but the
          // segment was too short. Detach the onstop handler so the
          // discarded recorder does not dispatch SPEECH_END, stop it
          // to release the encoder, and unwind the store.
          voiceDebug('speech abort')
          const rec = recorderRef.current
          recorderRef.current = null
          if (rec) {
            rec.onstop = null
            if (rec.state !== 'inactive') {
              try {
                rec.stop()
              } catch {
                // ignore
              }
            }
          }
          store.send({ type: 'SPEECH_ABORTED' })
        },
      })
      vad.start()
      vadRef.current = vad

      // Mirror responding-state into VAD barge-in mode so ambient
      // blips don't trigger speech-start during agent work.
      let prevLoggedState: string | null = null
      const stateSub = store.subscribe((snapshot) => {
        const s = snapshot.context.state
        if (s !== prevLoggedState) {
          voiceDebug('state', prevLoggedState, '->', s)
          prevLoggedState = s
        }
        vad.setBargeInMode(s === 'responding' || s === 'barge_in_pending')
      })
      stateSubRef.current = stateSub

      warmUpTimerRef.current = setTimeout(() => {
        store.send({ type: 'WARM_UP_DONE' })
        warmUpTimerRef.current = null
      }, WARM_UP_MS)

      track(SIDEPANEL_VOICE_MODE_OPENED_EVENT, { vadStrategy: vad.strategy })
      store.send({ type: 'OPEN' })
    } catch (err) {
      releaseResources()
      store.send({ type: 'ERROR', message: describeCaptureError(err) })
    }
  }

  const close = () => {
    track(SIDEPANEL_VOICE_MODE_CLOSED_EVENT)
    store.send({ type: 'CLOSE' })
  }

  const stopAgentActivity = () => {
    track(SIDEPANEL_VOICE_MODE_STOP_AGENT_EVENT)
    store.send({ type: 'STOP_AGENT' })
  }

  const retry = () => {
    // Error state reaches this point only after releaseResources()
    // ran in open()'s catch, so capture/vad/monitor refs are all
    // null. Dispatching a store-only RETRY would put the chip back
    // to "Listening" with no live capture behind it. Re-running
    // open() reacquires the mic and re-emits OPEN, which clears the
    // error chip naturally.
    void open()
  }

  // The api is constructed once via lazy useState and stays
  // referentially stable across renders. Callers passing this
  // object as a prop (e.g. ChatFooter -> VoiceMode) never see it
  // change identity, so memoized children skip cleanly.
  const openRef = useRef(open)
  const closeRef = useRef(close)
  const stopAgentRef = useRef(stopAgentActivity)
  const retryRef = useRef(retry)
  openRef.current = open
  closeRef.current = close
  stopAgentRef.current = stopAgentActivity
  retryRef.current = retry

  const [api] = useState<VoiceLoopApi>(() => ({
    store,
    interruptedMessageIds: interruptedIdsRef.current,
    open: () => openRef.current(),
    close: () => closeRef.current(),
    stopAgentActivity: () => stopAgentRef.current(),
    retry: () => retryRef.current(),
  }))

  return api
}

function lastAssistantId(messages: UIMessage[]): string | null {
  for (let i = messages.length - 1; i >= 0; i--) {
    if (messages[i]?.role === 'assistant') return messages[i].id
  }
  return null
}
