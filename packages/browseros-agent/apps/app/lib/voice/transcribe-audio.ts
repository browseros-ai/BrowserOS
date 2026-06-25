const GATEWAY_URL = 'https://llm.browseros.com'

const BIAS_PROMPT =
  'Transcript of a user dictating a chat message. Do not describe non-speech sounds.'

export interface TranscribeResult {
  text: string
  avgLogprob?: number
}

interface VerboseSegment {
  avg_logprob?: number
  no_speech_prob?: number
}

interface TranscribeResponse {
  text?: string
  segments?: VerboseSegment[]
}

export async function transcribeAudio(
  audioBlob: Blob,
): Promise<TranscribeResult> {
  const formData = new FormData()
  formData.append('file', audioBlob, 'recording.webm')
  // verbose_json gives us per-segment avg_logprob; if the gateway
  // ignores it we still get the plain text back and just skip the
  // confidence-based drop in the sanitizer.
  formData.append('response_format', 'verbose_json')
  // Lock Whisper to English. Without this, auto-detect flips short
  // utterances between scripts (e.g. English -> Punjabi/Hindi) and the
  // LLM mirrors the wrong script in its reply.
  formData.append('language', 'en')
  // temperature 0 makes Whisper deterministic and less prone to
  // hallucinating sound-tag descriptions over near-silent input.
  formData.append('temperature', '0')
  // A short decoder bias. Keep neutral; do not feed user history in
  // here or Whisper will complete the prompt as text rather than
  // transcribe the audio.
  formData.append('prompt', BIAS_PROMPT)

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
  return {
    text: result.text ?? '',
    avgLogprob: meanLogprob(result.segments),
  }
}

function meanLogprob(
  segments: VerboseSegment[] | undefined,
): number | undefined {
  if (!segments || segments.length === 0) return undefined
  let sum = 0
  let n = 0
  for (const s of segments) {
    if (typeof s.avg_logprob === 'number') {
      sum += s.avg_logprob
      n++
    }
  }
  return n === 0 ? undefined : sum / n
}
