// Dev-only voice tracing. Off by default. Flip the flag from a devtools
// console session (`window.__VOICE_DEBUG__ = true`) or set the build env
// `VITE_VOICE_DEBUG=1` to keep it on across reloads.

declare global {
  interface Window {
    __VOICE_DEBUG__?: boolean
  }
}

const envFlag =
  typeof import.meta !== 'undefined' &&
  (import.meta as ImportMeta & { env?: Record<string, string> }).env
    ?.VITE_VOICE_DEBUG === '1'

export function isVoiceDebugOn(): boolean {
  if (envFlag) return true
  return typeof window !== 'undefined' && window.__VOICE_DEBUG__ === true
}

export function voiceDebug(...args: unknown[]): void {
  if (!isVoiceDebugOn()) return
  // biome-ignore lint/suspicious/noConsole: dev-only debug logging
  console.debug('[voice]', ...args)
}
