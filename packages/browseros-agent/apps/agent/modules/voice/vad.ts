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
  readonly strategy: 'energy'
}

export interface VadOptions {
  silenceThresholdMs?: number
  minSpeechDurationMs?: number
}

const DEFAULT_SILENCE_MS = 600
const DEFAULT_MIN_SPEECH_MS = 200
const ENERGY_UPPER_THRESHOLD = 35
const ENERGY_LOWER_THRESHOLD = 18

// Energy-threshold VAD driven by the existing AudioLevelMonitor's
// aggregate amplitude. Speech start fires the first time the aggregate
// crosses the upper threshold; speech end fires after the aggregate
// stays below the lower threshold for `silenceThresholdMs`. Hysteresis
// between the two thresholds prevents flicker.
export function createVad(
  _capture: AudioCaptureHandle,
  monitor: AudioLevelMonitor,
  events: VadEvents,
  opts: VadOptions = {},
): VadHandle {
  const silenceMs = opts.silenceThresholdMs ?? DEFAULT_SILENCE_MS
  const minSpeechMs = opts.minSpeechDurationMs ?? DEFAULT_MIN_SPEECH_MS

  let isSpeaking = false
  let speechStartedAt = 0
  let silenceStartedAt = 0
  let active = false
  let unsubscribe: (() => void) | null = null

  const onSample = ({ aggregate }: { aggregate: number }) => {
    if (!active) return
    const now = performance.now()
    if (!isSpeaking) {
      if (aggregate >= ENERGY_UPPER_THRESHOLD) {
        isSpeaking = true
        speechStartedAt = now
        silenceStartedAt = 0
        events.onSpeechStart()
      }
      return
    }
    if (aggregate < ENERGY_LOWER_THRESHOLD) {
      if (silenceStartedAt === 0) silenceStartedAt = now
      if (now - silenceStartedAt >= silenceMs) {
        if (now - speechStartedAt >= minSpeechMs) {
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
    stop() {
      active = false
      unsubscribe?.()
      unsubscribe = null
      isSpeaking = false
      silenceStartedAt = 0
    },
  }
}
