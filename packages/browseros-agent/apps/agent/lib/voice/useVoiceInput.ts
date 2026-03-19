import { useEffect, useRef, useState } from 'react'

const GATEWAY_URL = 'https://llm.browseros.com'
const WAVEFORM_BAND_COUNT = 5

export interface VoiceInputState {
  isRecording: boolean
  isTranscribing: boolean
  audioLevels: number[]
  error: string | null
  onStartRecording: () => void
  onStopRecording: () => void
}

export interface UseVoiceInputReturn {
  isRecording: boolean
  isTranscribing: boolean
  transcript: string
  audioLevel: number
  audioLevels: number[]
  error: string | null
  startRecording: () => Promise<boolean>
  stopRecording: () => Promise<void>
  clearTranscript: () => void
}

const EMPTY_LEVELS = Array(WAVEFORM_BAND_COUNT).fill(0)

interface TranscribeResponse {
  text: string
}

async function transcribeAudio(audioBlob: Blob): Promise<string> {
  const formData = new FormData()
  formData.append('file', audioBlob, 'recording.webm')
  formData.append('response_format', 'json')

  const response = await fetch(`${GATEWAY_URL}/api/transcribe`, {
    method: 'POST',
    body: formData,
    signal: AbortSignal.timeout(30_000),
  })

  if (!response.ok) {
    const errorBody: { error?: string } = await response
      .json()
      .catch(() => ({ error: 'Transcription failed' }))
    throw new Error(
      errorBody.error || `Transcription failed: ${response.status}`,
    )
  }

  const result: TranscribeResponse = await response.json()
  return result.text || ''
}

export function useVoiceInput(): UseVoiceInputReturn {
  const [isRecording, setIsRecording] = useState(false)
  const [isTranscribing, setIsTranscribing] = useState(false)
  const [transcript, setTranscript] = useState('')
  const [audioLevel, setAudioLevel] = useState(0)
  const [audioLevels, setAudioLevels] = useState<number[]>(EMPTY_LEVELS)
  const [error, setError] = useState<string | null>(null)

  const mediaRecorderRef = useRef<MediaRecorder | null>(null)
  const streamRef = useRef<MediaStream | null>(null)
  const chunksRef = useRef<Blob[]>([])
  const audioContextRef = useRef<AudioContext | null>(null)
  const analyserRef = useRef<AnalyserNode | null>(null)
  const animationFrameRef = useRef<number | null>(null)

  const stopAudioLevelMonitoring = () => {
    if (animationFrameRef.current) {
      cancelAnimationFrame(animationFrameRef.current)
      animationFrameRef.current = null
    }
    if (audioContextRef.current?.state !== 'closed') {
      audioContextRef.current?.close()
    }
    audioContextRef.current = null
    analyserRef.current = null
    setAudioLevel(0)
    setAudioLevels(EMPTY_LEVELS)
  }

  // biome-ignore lint/correctness/useExhaustiveDependencies: cleanup only needs to run on unmount
  useEffect(() => {
    return () => {
      streamRef.current?.getTracks().forEach((track) => {
        track.stop()
      })
      if (mediaRecorderRef.current?.state === 'recording') {
        mediaRecorderRef.current.stop()
      }
      stopAudioLevelMonitoring()
    }
  }, [])

  const startAudioLevelMonitoring = (stream: MediaStream) => {
    const audioContext = new AudioContext()
    const analyser = audioContext.createAnalyser()
    analyser.fftSize = 256

    const source = audioContext.createMediaStreamSource(stream)
    source.connect(analyser)

    audioContextRef.current = audioContext
    analyserRef.current = analyser

    const updateLevel = () => {
      if (!analyserRef.current) return

      const dataArray = new Uint8Array(analyserRef.current.fftSize)
      analyserRef.current.getByteTimeDomainData(dataArray)

      const binCount = dataArray.length
      const levels: number[] = []
      let totalPeak = 0

      for (let band = 0; band < WAVEFORM_BAND_COUNT; band++) {
        const start = Math.floor((band / WAVEFORM_BAND_COUNT) * binCount)
        const end = Math.floor(((band + 1) / WAVEFORM_BAND_COUNT) * binCount)
        let peak = 0
        for (let j = start; j < end; j++) {
          const amplitude = Math.abs(dataArray[j] - 128)
          if (amplitude > peak) peak = amplitude
        }
        const normalized = Math.round(Math.min(100, (peak / 50) * 100))
        levels.push(normalized)
        totalPeak += normalized
      }

      setAudioLevels(levels)
      setAudioLevel(Math.round(totalPeak / WAVEFORM_BAND_COUNT))

      animationFrameRef.current = requestAnimationFrame(updateLevel)
    }

    updateLevel()
  }

  const startRecording = async (): Promise<boolean> => {
    try {
      setError(null)
      setTranscript('')
      chunksRef.current = []

      const stream = await navigator.mediaDevices.getUserMedia({
        audio: {
          channelCount: 1,
          sampleRate: 16000,
          echoCancellation: true,
          noiseSuppression: true,
        },
      })

      streamRef.current = stream
      startAudioLevelMonitoring(stream)

      const mimeType = MediaRecorder.isTypeSupported('audio/webm;codecs=opus')
        ? 'audio/webm;codecs=opus'
        : 'audio/webm'

      const mediaRecorder = new MediaRecorder(stream, { mimeType })
      mediaRecorderRef.current = mediaRecorder

      mediaRecorder.ondataavailable = (e) => {
        if (e.data.size > 0) {
          chunksRef.current.push(e.data)
        }
      }

      mediaRecorder.start(250)
      setIsRecording(true)
      return true
    } catch (err) {
      streamRef.current?.getTracks().forEach((track) => {
        track.stop()
      })
      streamRef.current = null
      stopAudioLevelMonitoring()

      if (err instanceof Error) {
        if (err.name === 'NotAllowedError') {
          setError('Microphone permission denied')
        } else if (err.name === 'NotFoundError') {
          setError('No microphone found')
        } else {
          setError(err.message)
        }
      } else {
        setError('Failed to start recording')
      }
      return false
    }
  }

  const stopRecording = async () => {
    const mediaRecorder = mediaRecorderRef.current

    if (!mediaRecorder || mediaRecorder.state === 'inactive') {
      return
    }

    await new Promise<void>((resolve) => {
      mediaRecorder.onstop = () => resolve()
      mediaRecorder.stop()
    })

    streamRef.current?.getTracks().forEach((track) => {
      track.stop()
    })
    streamRef.current = null
    stopAudioLevelMonitoring()
    setIsRecording(false)

    const audioBlob = new Blob(chunksRef.current, { type: 'audio/webm' })
    chunksRef.current = []

    if (audioBlob.size === 0) {
      setError('No audio recorded')
      return
    }

    setIsTranscribing(true)
    try {
      const text = await transcribeAudio(audioBlob)
      if (text.trim()) {
        setTranscript(text.trim())
      } else {
        setError('No speech detected')
      }
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Transcription failed')
    } finally {
      setIsTranscribing(false)
    }
  }

  const clearTranscript = () => {
    setTranscript('')
    setError(null)
  }

  return {
    isRecording,
    isTranscribing,
    transcript,
    audioLevel,
    audioLevels,
    error,
    startRecording,
    stopRecording,
    clearTranscript,
  }
}
