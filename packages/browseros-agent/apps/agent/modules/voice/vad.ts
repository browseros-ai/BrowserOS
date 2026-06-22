import type { AudioCaptureHandle } from './audio-capture'
import type { AudioLevelMonitor } from './audio-level-monitor'
import { createSileroVad } from './vad.silero'
import { voiceDebug } from './voice-debug'

export interface VadEvents {
  onSpeechStart(): void
  onSpeechEnd(): void
  // Fired when the VAD speculatively reported speech-start but the
  // segment never met the minimum duration. The hook must stop any
  // recorder it started on speech-start; no SPEECH_END goes to the
  // store. Silero never fires this (its minSpeechFrames filter runs
  // before any callback). Energy uses it because it has no buffer
  // to confirm minimum duration before reporting speech-start.
  onSpeechAbort?(): void
}

export interface VadHandle {
  start(): void
  pause(): void
  resume(): void
  stop(): void
  // Raise the bar while the agent is responding. Short blips that
  // would pass the normal listening floor are rejected in this mode
  // so notifications, keyboard clicks, and brief coughs never reach
  // the transcription stage during agent work. Silero's parameters
  // are baked in at construction, so the energy fallback is the only
  // strategy that actually uses this flag; for Silero this is a no-op.
  setBargeInMode(active: boolean): void
  readonly strategy: 'energy' | 'silero'
}

export interface VadOptions {
  strategy?: 'silero' | 'energy'
  silenceThresholdMs?: number
  minSpeechDurationMs?: number
  bargeInMinSpeechMs?: number
  upperThreshold?: number
  lowerThreshold?: number
  bargeInUpperThreshold?: number
}

// Tolerate mid-sentence pauses ("...what happens... if I pause") as one
// turn. Industry voice modes settle around 1-1.5s; 1200ms feels natural
// without making real turn-ends feel laggy.
const DEFAULT_SILENCE_MS = 1200
const DEFAULT_MIN_SPEECH_MS = 400
const DEFAULT_BARGE_IN_MIN_SPEECH_MS = 700
const DEFAULT_UPPER = 50
const DEFAULT_LOWER = 30
const DEFAULT_BARGE_IN_UPPER = 60

// Default to Silero (ML-based speech detection). Energy is the soft
// fallback when Silero can't load (asset fetch failure, MV3 CSP weirdness,
// onnxruntime init crash). Callers can also force the energy path by
// passing `strategy: 'energy'`.
export async function createVad(
  capture: AudioCaptureHandle,
  monitor: AudioLevelMonitor,
  events: VadEvents,
  opts: VadOptions = {},
): Promise<VadHandle> {
  const strategy = opts.strategy ?? 'silero'
  if (strategy === 'silero') {
    try {
      const vad = await createSileroVad(capture, events, opts)
      voiceDebug('vad strategy', 'silero')
      return vad
    } catch (err) {
      voiceDebug('silero load failed, falling back to energy', err)
    }
  }
  voiceDebug('vad strategy', 'energy')
  return createEnergyVad(monitor, events, opts)
}

// Energy-threshold VAD driven by the existing AudioLevelMonitor's
// aggregate amplitude. Speech start fires the first time the aggregate
// crosses the upper threshold; speech end fires after the aggregate
// stays below the lower threshold for `silenceThresholdMs`. Hysteresis
// between the two thresholds prevents flicker. Barge-in mode raises
// the upper threshold and the minimum speech duration so the agent is
// not cancelled by ambient blips.
function createEnergyVad(
  monitor: AudioLevelMonitor,
  events: VadEvents,
  opts: VadOptions = {},
): VadHandle {
  const silenceMs = opts.silenceThresholdMs ?? DEFAULT_SILENCE_MS
  const minSpeechMs = opts.minSpeechDurationMs ?? DEFAULT_MIN_SPEECH_MS
  const bargeMinSpeechMs =
    opts.bargeInMinSpeechMs ?? DEFAULT_BARGE_IN_MIN_SPEECH_MS
  const upper = opts.upperThreshold ?? DEFAULT_UPPER
  const lower = opts.lowerThreshold ?? DEFAULT_LOWER
  const bargeUpper = opts.bargeInUpperThreshold ?? DEFAULT_BARGE_IN_UPPER

  let isSpeaking = false
  let speechStartedAt = 0
  let silenceStartedAt = 0
  let active = false
  let bargeInMode = false
  let unsubscribe: (() => void) | null = null

  const currentUpper = () => (bargeInMode ? bargeUpper : upper)
  const currentMinSpeech = () => (bargeInMode ? bargeMinSpeechMs : minSpeechMs)

  const onSample = ({ aggregate }: { aggregate: number }) => {
    if (!active) return
    const now = performance.now()
    if (!isSpeaking) {
      if (aggregate >= currentUpper()) {
        isSpeaking = true
        speechStartedAt = now
        silenceStartedAt = 0
        events.onSpeechStart()
      }
      return
    }
    if (aggregate < lower) {
      if (silenceStartedAt === 0) silenceStartedAt = now
      if (now - silenceStartedAt >= silenceMs) {
        if (now - speechStartedAt >= currentMinSpeech()) {
          events.onSpeechEnd()
        } else {
          // Speech-start fired but the segment never reached the
          // minimum duration. Signal the hook so it stops the
          // recorder it speculatively started; no SPEECH_END is
          // dispatched, the loop quietly returns to listening.
          events.onSpeechAbort?.()
        }
        isSpeaking = false
        silenceStartedAt = 0
      }
    } else {
      silenceStartedAt = 0
    }
  }

  return {
    strategy: 'energy',
    start() {
      if (active) return
      active = true
      unsubscribe = monitor.subscribe(onSample)
    },
    pause() {
      active = false
    },
    resume() {
      active = true
    },
    setBargeInMode(activeFlag: boolean) {
      bargeInMode = activeFlag
    },
    stop() {
      active = false
      unsubscribe?.()
      unsubscribe = null
      isSpeaking = false
      silenceStartedAt = 0
      bargeInMode = false
    },
  }
}
