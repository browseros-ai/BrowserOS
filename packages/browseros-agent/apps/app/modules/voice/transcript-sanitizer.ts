export type DropReason =
  | 'empty'
  | 'hallucination_only'
  | 'short_noise'
  | 'low_confidence'

export interface SanitizeOptions {
  avgLogprob?: number
  lowConfidenceThreshold?: number
}

export type SanitizeResult =
  | { action: 'send'; text: string }
  | { action: 'drop'; reason: DropReason }

// Match the innermost group first so nested `((music))` peels correctly
// across iterations. A `[^()]*` body cannot itself contain parens, so
// the regex picks the deepest balanced pair on each pass.
const SOUND_TAG_RE = /\([^()]*\)|\[[^[\]]*\]/g

const HALLUCINATION_PHRASES = Object.freeze(
  new Set([
    'thank you for watching',
    'thanks for watching',
    'thank you for listening',
    'thanks for listening',
    'subscribe to my channel',
    'please subscribe',
    'like and subscribe',
    'see you next time',
    'bye',
    'goodbye',
    'beep',
    'beeps',
    'music',
    'beatbox',
    'beatbox sounds',
    'computerized voice',
    'silence',
    'no audio',
    'inaudible',
    'background noise',
  ]),
)

const LEGIT_SHORT_COMMANDS = Object.freeze(
  new Set([
    'yes',
    'no',
    'stop',
    'ok',
    'okay',
    'cancel',
    'continue',
    'wait',
    'pause',
    'resume',
    'quit',
    'done',
    'go',
    'next',
    'back',
  ]),
)

export function stripSoundTags(input: string): string {
  let prev = input
  let next = prev.replace(SOUND_TAG_RE, ' ')
  while (next !== prev) {
    prev = next
    next = prev.replace(SOUND_TAG_RE, ' ')
  }
  return next.replace(/\s+/g, ' ').trim()
}

export function isHallucinationPhrase(input: string): boolean {
  const normalized = input
    .toLowerCase()
    .replace(/[.,!?;:"'`]/g, '')
    .replace(/\s+/g, ' ')
    .trim()
  if (!normalized) return false
  return HALLUCINATION_PHRASES.has(normalized)
}

export function sanitize(
  input: string,
  opts: SanitizeOptions = {},
): SanitizeResult {
  const threshold = opts.lowConfidenceThreshold ?? -1.0
  if (typeof opts.avgLogprob === 'number' && opts.avgLogprob < threshold) {
    return { action: 'drop', reason: 'low_confidence' }
  }

  const stripped = stripSoundTags(input)
  if (!stripped) return { action: 'drop', reason: 'empty' }

  if (isHallucinationPhrase(stripped)) {
    return { action: 'drop', reason: 'hallucination_only' }
  }

  const words = stripped.split(/\s+/).filter(Boolean)
  if (words.length === 1) {
    const word = words[0].toLowerCase().replace(/[.,!?;:"'`]/g, '')
    if (!LEGIT_SHORT_COMMANDS.has(word)) {
      return { action: 'drop', reason: 'short_noise' }
    }
  }

  return { action: 'send', text: stripped }
}
