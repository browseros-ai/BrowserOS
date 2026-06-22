import { useEffect, useRef, useState } from 'react'
import { transcribeAudio } from '@/lib/voice/transcribe-audio'
import {
  type AudioCaptureHandle,
  describeCaptureError,
  openAudioCapture,
} from './audio-capture'
import {
  type AudioLevelMonitor,
  createAudioLevelMonitor,
  emptySample,
} from './audio-level-monitor'

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

const EMPTY_LEVELS = emptySample(WAVEFORM_BAND_COUNT).levels

export function useVoiceInput(): UseVoiceInputReturn {
  const [isRecording, setIsRecording] = useState(false)
  const [isTranscribing, setIsTranscribing] = useState(false)
  const [transcript, setTranscript] = useState('')
  const [audioLevel, setAudioLevel] = useState(0)
  const [audioLevels, setAudioLevels] = useState<number[]>(EMPTY_LEVELS)
  const [error, setError] = useState<string | null>(null)

  const captureRef = useRef<AudioCaptureHandle | null>(null)
  const monitorRef = useRef<AudioLevelMonitor | null>(null)
  const mediaRecorderRef = useRef<MediaRecorder | null>(null)
  const chunksRef = useRef<Blob[]>([])

  const releaseAll = () => {
    monitorRef.current?.stop()
    monitorRef.current = null
    captureRef.current?.close()
    captureRef.current = null
    setAudioLevel(0)
    setAudioLevels(EMPTY_LEVELS)
  }

  // biome-ignore lint/correctness/useExhaustiveDependencies: cleanup only needs to run on unmount
  useEffect(() => {
    return () => {
      if (mediaRecorderRef.current?.state === 'recording') {
        mediaRecorderRef.current.stop()
      }
      releaseAll()
    }
  }, [])

  const startRecording = async (): Promise<boolean> => {
    try {
      setError(null)
      setTranscript('')
      chunksRef.current = []

      const capture = await openAudioCapture()
      captureRef.current = capture

      const monitor = createAudioLevelMonitor({
        bandCount: WAVEFORM_BAND_COUNT,
      })
      monitor.subscribe((sample) => {
        setAudioLevels(sample.levels)
        setAudioLevel(sample.aggregate)
      })
      monitor.start(capture.analyser)
      monitorRef.current = monitor

      const mimeType = MediaRecorder.isTypeSupported('audio/webm;codecs=opus')
        ? 'audio/webm;codecs=opus'
        : 'audio/webm'

      const mediaRecorder = new MediaRecorder(capture.stream, { mimeType })
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
      releaseAll()
      setError(describeCaptureError(err))
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

    releaseAll()
    setIsRecording(false)

    const audioBlob = new Blob(chunksRef.current, { type: 'audio/webm' })
    chunksRef.current = []

    if (audioBlob.size === 0) {
      setError('No audio recorded')
      return
    }

    setIsTranscribing(true)
    try {
      const { text } = await transcribeAudio(audioBlob)
      const trimmed = text.trim()
      if (trimmed) {
        setTranscript(trimmed)
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
