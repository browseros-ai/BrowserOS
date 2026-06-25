import type { AudioCaptureHandle } from './audio-capture'
import type { VadEvents, VadHandle, VadOptions } from './vad'

// Resolve a public/ asset URL inside the extension. In MV3 contexts
// chrome.runtime.getURL is the only resolver that works; web preview
// falls back to a root-relative path.
function vadAssetUrl(path: string): string {
  if (typeof chrome !== 'undefined' && chrome.runtime?.getURL) {
    return chrome.runtime.getURL(path)
  }
  return `/${path}`
}

export async function createSileroVad(
  capture: AudioCaptureHandle,
  events: VadEvents,
  opts: VadOptions,
): Promise<VadHandle> {
  // Dynamic-import keeps onnxruntime + the worklet bundle off the
  // critical path. Side panel opens fast even if Silero is slow to
  // fetch.
  const [{ MicVAD }, ort] = await Promise.all([
    import('@ricky0123/vad-web'),
    import('onnxruntime-web'),
  ])

  ort.env.wasm.wasmPaths = vadAssetUrl('onnxruntime/')

  const frameSamples = 1536
  const sampleRate = 16000
  const msPerFrame = (frameSamples / sampleRate) * 1000
  const minSpeechMs = opts.minSpeechDurationMs ?? 400
  // Give natural mid-sentence pauses room to breathe. 1200ms catches
  // "what happens... if there's a delay" as one turn instead of two.
  const silenceMs = opts.silenceThresholdMs ?? 1200
  const minSpeechFrames = Math.max(1, Math.round(minSpeechMs / msPerFrame))
  const redemptionFrames = Math.max(1, Math.round(silenceMs / msPerFrame))

  const micVad = await MicVAD.new({
    stream: capture.stream,
    workletURL: vadAssetUrl('vad/vad.worklet.bundle.min.js'),
    modelURL: vadAssetUrl('vad/silero_vad.onnx'),
    modelFetcher: async (url: string) => {
      const res = await fetch(url)
      if (!res.ok) throw new Error(`Failed to fetch VAD model: ${res.status}`)
      return res.arrayBuffer()
    },
    // Silero is already trained to distinguish speech from non-speech.
    // The store-level two-stage barge-in and the transcript sanitizer
    // handle the rest of the noise rejection, so we keep the model
    // thresholds at sensible defaults rather than juggling two sets.
    positiveSpeechThreshold: 0.55,
    negativeSpeechThreshold: 0.35,
    redemptionFrames,
    minSpeechFrames,
    preSpeechPadFrames: 2,
    frameSamples,
    onSpeechStart: () => events.onSpeechStart(),
    onSpeechEnd: () => events.onSpeechEnd(),
  })

  return {
    strategy: 'silero',
    start: () => micVad.start(),
    pause: () => micVad.pause(),
    resume: () => micVad.start(),
    // Silero parameters are baked in at construction. Barge-in safety
    // lives in the store-level two-stage flow and the sanitizer, not
    // in dynamic Silero retuning.
    setBargeInMode: () => undefined,
    stop: () => micVad.destroy(),
  }
}
