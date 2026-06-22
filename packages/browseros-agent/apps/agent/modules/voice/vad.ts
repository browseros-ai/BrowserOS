import type { AudioCaptureHandle } from './audio-capture'
import type { AudioLevelMonitor } from './audio-level-monitor'

export interface VadEvents {
  onSpeechStart(): void
  onSpeechEnd(samples: Float32Array): void
}

export interface VadHandle {
  start(): void
  pause(): void
  resume(): void
  stop(): void
  readonly strategy: 'silero' | 'energy'
}

export interface VadOptions {
  silenceThresholdMs?: number
  minSpeechDurationMs?: number
  strategy?: 'silero' | 'energy'
}

const DEFAULT_SILENCE_MS = 600
const DEFAULT_MIN_SPEECH_MS = 200
const ENERGY_UPPER_THRESHOLD = 35
const ENERGY_LOWER_THRESHOLD = 18

export async function createVad(
  capture: AudioCaptureHandle,
  monitor: AudioLevelMonitor,
  events: VadEvents,
  opts: VadOptions = {},
): Promise<VadHandle> {
  // Default to energy detection in v1 to avoid the AudioWorklet + ONNX
  // wasm load path, which has crashed the renderer process in MV3
  // contexts. Silero stays opt-in via opts.strategy until that load
  // path is hardened.
  const strategy = opts.strategy ?? 'energy'
  if (strategy === 'silero') {
    try {
      return await createSileroVad(capture, events, opts)
    } catch (err) {
      // biome-ignore lint/suspicious/noConsole: surface the fallback decision so QA can see when Silero failed to load
      console.warn(
        'Silero VAD failed to initialize, falling back to energy',
        err,
      )
      return createEnergyVad(monitor, events, opts)
    }
  }
  return createEnergyVad(monitor, events, opts)
}

function vadAssetUrl(path: string): string {
  if (typeof chrome !== 'undefined' && chrome.runtime?.getURL) {
    return chrome.runtime.getURL(path)
  }
  return `/${path}`
}

async function createSileroVad(
  capture: AudioCaptureHandle,
  events: VadEvents,
  opts: VadOptions,
): Promise<VadHandle> {
  const [{ MicVAD }, ort] = await Promise.all([
    import('@ricky0123/vad-web'),
    import('onnxruntime-web'),
  ])

  ort.env.wasm.wasmPaths = vadAssetUrl('onnxruntime/')

  const silenceMs = opts.silenceThresholdMs ?? DEFAULT_SILENCE_MS
  const minSpeechMs = opts.minSpeechDurationMs ?? DEFAULT_MIN_SPEECH_MS
  const frameSamples = 1536
  const sampleRate = 16000
  const framesPerMs = sampleRate / 1000 / frameSamples
  const redemptionFrames = Math.max(1, Math.round(silenceMs * framesPerMs))
  const minSpeechFrames = Math.max(1, Math.round(minSpeechMs * framesPerMs))

  const micVad = await MicVAD.new({
    stream: capture.stream,
    workletURL: vadAssetUrl('vad/vad.worklet.bundle.min.js'),
    modelURL: vadAssetUrl('vad/silero_vad.onnx'),
    modelFetcher: async (url: string) => {
      const res = await fetch(url)
      if (!res.ok) throw new Error(`Failed to fetch VAD model: ${res.status}`)
      return res.arrayBuffer()
    },
    positiveSpeechThreshold: 0.5,
    negativeSpeechThreshold: 0.35,
    redemptionFrames,
    minSpeechFrames,
    frameSamples,
    onSpeechStart: () => events.onSpeechStart(),
    onSpeechEnd: (samples: Float32Array) => events.onSpeechEnd(samples),
  })

  return {
    strategy: 'silero',
    start: () => micVad.start(),
    pause: () => micVad.pause(),
    resume: () => micVad.start(),
    stop: () => micVad.destroy(),
  }
}

function createEnergyVad(
  monitor: AudioLevelMonitor,
  events: VadEvents,
  opts: VadOptions,
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
          events.onSpeechEnd(new Float32Array(0))
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

export function pcmFloat32ToWavBlob(
  samples: Float32Array,
  sampleRate = 16000,
): Blob {
  const buffer = new ArrayBuffer(44 + samples.length * 2)
  const view = new DataView(buffer)
  writeAscii(view, 0, 'RIFF')
  view.setUint32(4, 36 + samples.length * 2, true)
  writeAscii(view, 8, 'WAVE')
  writeAscii(view, 12, 'fmt ')
  view.setUint32(16, 16, true)
  view.setUint16(20, 1, true)
  view.setUint16(22, 1, true)
  view.setUint32(24, sampleRate, true)
  view.setUint32(28, sampleRate * 2, true)
  view.setUint16(32, 2, true)
  view.setUint16(34, 16, true)
  writeAscii(view, 36, 'data')
  view.setUint32(40, samples.length * 2, true)
  for (let i = 0; i < samples.length; i++) {
    const s = Math.max(-1, Math.min(1, samples[i]))
    view.setInt16(44 + i * 2, s < 0 ? s * 0x8000 : s * 0x7fff, true)
  }
  return new Blob([buffer], { type: 'audio/wav' })
}

function writeAscii(view: DataView, offset: number, text: string) {
  for (let i = 0; i < text.length; i++) {
    view.setUint8(offset + i, text.charCodeAt(i))
  }
}
