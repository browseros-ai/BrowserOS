import type { AudioCaptureHandle } from './audio-capture'
import type { AudioLevelMonitor } from './audio-level-monitor'

export interface VadEvents {
  onSpeechStart(): void
  onSpeechEnd(): void
}

export interface VadHandle {
  start(): void
  pause(): void
  resume(): void
  stop(): void
  // Raise the bar while the agent is responding. Short blips that
  // would pass the normal listening floor are rejected in this mode
  // so notifications, keyboard clicks, and brief coughs never reach
  // the transcription stage during agent work.
  setBargeInMode(active: boolean): void
  readonly strategy: 'energy' | 'silero'
}

export interface VadOptions {
  silenceThresholdMs?: number
  minSpeechDurationMs?: number
  bargeInMinSpeechMs?: number
  upperThreshold?: number
  lowerThreshold?: number
  bargeInUpperThreshold?: number
}

const DEFAULT_SILENCE_MS = 700
const DEFAULT_MIN_SPEECH_MS = 400
const DEFAULT_BARGE_IN_MIN_SPEECH_MS = 700
const DEFAULT_UPPER = 50
const DEFAULT_LOWER = 30
const DEFAULT_BARGE_IN_UPPER = 60

// Energy-threshold VAD driven by the existing AudioLevelMonitor's
// aggregate amplitude. Speech start fires the first time the aggregate
// crosses the upper threshold; speech end fires after the aggregate
// stays below the lower threshold for `silenceThresholdMs`. Hysteresis
// between the two thresholds prevents flicker. Barge-in mode raises
// the upper threshold and the minimum speech duration so the agent is
// not cancelled by ambient blips.
export function createVad(
  _capture: AudioCaptureHandle,
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
