import { useSelector } from '@xstate/store/react'
import type { UIMessage } from 'ai'
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
import { createVad, pcmFloat32ToWavBlob, type VadHandle } from './vad'
import { createVoiceLoopStore } from './voice-loop.store'
import type { VoiceLoopApi } from './voice-types'

const WARM_UP_MS = 800
const WAVEFORM_BAND_COUNT = 5

interface ChatSessionLike {
  sendMessage: (params: { text: string }) => void
  stop: () => void
  status: string
  messages: UIMessage[]
}

export interface UseVoiceLoopOptions {
  chatSession: ChatSessionLike
}

export function useVoiceLoop(opts: UseVoiceLoopOptions): VoiceLoopApi {
  const [store] = useState(() => createVoiceLoopStore())
  const state = useSelector(store, (s) => s.context.state)
  const audioLevels = useSelector(store, (s) => s.context.audioLevels)
  const errorMessage = useSelector(store, (s) => s.context.errorMessage)
  const isBargingIn = useSelector(store, (s) => s.context.isBargingIn)
  const [isWarmingUp, setIsWarmingUp] = useState(false)
  const [interruptedMessageIds, setInterruptedMessageIds] = useState<
    ReadonlySet<string>
  >(() => new Set())

  const captureRef = useRef<AudioCaptureHandle | null>(null)
  const monitorRef = useRef<AudioLevelMonitor | null>(null)
  const vadRef = useRef<VadHandle | null>(null)
  const recorderRef = useRef<MediaRecorder | null>(null)
  const transcribeAbortRef = useRef<AbortController | null>(null)
  const warmUpTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const messagesRef = useRef(opts.chatSession.messages)
  messagesRef.current = opts.chatSession.messages
  const prevStatusRef = useRef(opts.chatSession.status)

  useEffect(() => {
    const prev = prevStatusRef.current
    const next = opts.chatSession.status
    prevStatusRef.current = next
    if (prev === 'streaming' && next !== 'streaming') {
      store.send({ type: 'CHAT_STREAMING_ENDED' })
    }
  }, [opts.chatSession.status, store])

  // biome-ignore lint/correctness/useExhaustiveDependencies: releaseResources is a stable local helper; depending on it would re-subscribe emits on every render
  useEffect(() => {
    const sendMessage = opts.chatSession.sendMessage
    const stopChat = opts.chatSession.stop
    const subs = [
      store.on('runTranscribe', async ({ blob }) => {
        transcribeAbortRef.current?.abort()
        const ac = new AbortController()
        transcribeAbortRef.current = ac
        try {
          const text = await transcribeAudio(blob)
          if (ac.signal.aborted) return
          const trimmed = text.trim()
          if (!trimmed) {
            track(SIDEPANEL_VOICE_MODE_TRANSCRIBE_FAILED_EVENT, {
              reason: 'empty',
            })
            store.send({
              type: 'TRANSCRIBE_FAIL',
              message: 'No speech detected',
            })
            return
          }
          track(SIDEPANEL_VOICE_MODE_TURN_CAPTURED_EVENT, {
            chars: trimmed.length,
          })
          store.send({ type: 'TRANSCRIBE_OK', text: trimmed })
        } catch (err) {
          if (ac.signal.aborted) return
          const message =
            err instanceof Error ? err.message : 'Transcription failed'
          track(SIDEPANEL_VOICE_MODE_TRANSCRIBE_FAILED_EVENT, {
            reason: 'error',
          })
          store.send({ type: 'TRANSCRIBE_FAIL', message })
        }
      }),
      store.on('sendChatMessage', ({ text }) => {
        sendMessage({ text })
      }),
      store.on('cancelChatStream', () => {
        stopChat()
      }),
      store.on('markLastAssistantInterrupted', () => {
        const last = lastAssistantId(messagesRef.current)
        if (!last) return
        setInterruptedMessageIds((prev) => {
          if (prev.has(last)) return prev
          const next = new Set(prev)
          next.add(last)
          return next
        })
      }),
      store.on('releaseCapture', () => {
        releaseResources()
      }),
    ]
    return () => {
      for (const s of subs) s.unsubscribe()
    }
  }, [opts.chatSession.sendMessage, opts.chatSession.stop, store])

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
    setIsWarmingUp(false)
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
      monitor.subscribe((sample) => {
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
          if (store.getSnapshot().context.state === 'responding') {
            track(SIDEPANEL_VOICE_MODE_BARGE_IN_EVENT)
          }
          startTurnRecorder()
          store.send({ type: 'SPEECH_START' })
        },
        onSpeechEnd: (samples) => {
          if (samples.length > 0) {
            // Silero gave us a complete PCM segment; emit WAV directly.
            const rec = recorderRef.current
            recorderRef.current = null
            if (rec && rec.state !== 'inactive') {
              try {
                rec.stop()
              } catch {
                // ignore
              }
            }
            store.send({
              type: 'SPEECH_END',
              blob: pcmFloat32ToWavBlob(samples),
            })
            return
          }
          // Energy strategy: stop the per-turn recorder; its onstop
          // dispatches SPEECH_END with the freshly framed WebM blob.
          const rec = recorderRef.current
          recorderRef.current = null
          if (rec && rec.state !== 'inactive') {
            rec.stop()
          }
        },
      })
      vad.start()
      vadRef.current = vad

      setIsWarmingUp(true)
      warmUpTimerRef.current = setTimeout(() => {
        setIsWarmingUp(false)
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
    store.send({ type: 'RETRY' })
  }

  return {
    state,
    audioLevels,
    errorMessage,
    isBargingIn,
    isWarmingUp,
    interruptedMessageIds,
    open,
    close,
    stopAgentActivity,
    retry,
  }
}

function lastAssistantId(messages: UIMessage[]): string | null {
  for (let i = messages.length - 1; i >= 0; i--) {
    if (messages[i]?.role === 'assistant') return messages[i].id
  }
  return null
}
