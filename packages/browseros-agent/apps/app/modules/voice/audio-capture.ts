export interface AudioCaptureHandle {
  readonly stream: MediaStream
  readonly audioContext: AudioContext
  readonly analyser: AnalyserNode
  close(): void
}

const DEFAULT_CONSTRAINTS: MediaTrackConstraints = {
  channelCount: 1,
  sampleRate: 16000,
  echoCancellation: true,
  noiseSuppression: true,
}

export async function openAudioCapture(
  constraints: MediaTrackConstraints = DEFAULT_CONSTRAINTS,
): Promise<AudioCaptureHandle> {
  const stream = await navigator.mediaDevices.getUserMedia({
    audio: constraints,
  })
  let audioContext: AudioContext
  let analyser: AnalyserNode
  try {
    audioContext = new AudioContext()
    analyser = audioContext.createAnalyser()
    analyser.fftSize = 256
    const source = audioContext.createMediaStreamSource(stream)
    source.connect(analyser)
  } catch (err) {
    stream.getTracks().forEach((t) => {
      t.stop()
    })
    throw err
  }

  let closed = false
  return {
    stream,
    audioContext,
    analyser,
    close() {
      if (closed) return
      closed = true
      stream.getTracks().forEach((t) => {
        t.stop()
      })
      if (audioContext.state !== 'closed') {
        audioContext.close().catch(() => undefined)
      }
    },
  }
}

export function describeCaptureError(err: unknown): string {
  if (!(err instanceof Error)) return 'Failed to start recording'
  if (err.name === 'NotAllowedError') return 'Microphone permission denied'
  if (err.name === 'NotFoundError') return 'No microphone found'
  return err.message
}
