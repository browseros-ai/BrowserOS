const GATEWAY_URL = 'https://llm.browseros.com'

interface TranscribeResponse {
  text: string
}

export async function transcribeAudio(audioBlob: Blob): Promise<string> {
  const formData = new FormData()
  formData.append('file', audioBlob, 'recording.webm')
  formData.append('response_format', 'json')
  // Lock Whisper to English. Without this, auto-detect flips short
  // utterances between scripts (e.g. English -> Punjabi/Hindi) and the
  // LLM mirrors the wrong script in its reply.
  formData.append('language', 'en')

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
