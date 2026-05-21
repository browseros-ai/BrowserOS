export async function transcribeAudio(audioBlob: Blob): Promise<string> {
  // Security Hardening: Disabled voice recording upload to BrowserOS Cloud
  console.warn('Voice transcription is disabled for security reasons.')
  throw new Error('Voice transcription is disabled in this hardened version.')
}
