import { describe, expect, it } from 'bun:test'
import {
  isHallucinationPhrase,
  sanitize,
  stripSoundTags,
} from './transcript-sanitizer'

describe('stripSoundTags', () => {
  it('removes parenthetical sound tags', () => {
    expect(stripSoundTags('Hello (beeps) world')).toBe('Hello world')
  })

  it('removes bracketed sound tags', () => {
    expect(stripSoundTags('Hello [music] world')).toBe('Hello world')
  })

  it('removes multiple tags in one string', () => {
    expect(
      stripSoundTags(
        'Explain photosynthesis (beeps) with the scientific method (computerized voice).',
      ),
    ).toBe('Explain photosynthesis with the scientific method .')
  })

  it('removes nested tags iteratively', () => {
    expect(stripSoundTags('foo ((music)) bar')).toBe('foo bar')
  })

  it('returns empty string for tag-only input', () => {
    expect(stripSoundTags('(beatbox sounds)')).toBe('')
    expect(stripSoundTags('[Music]')).toBe('')
  })

  it('collapses whitespace and trims', () => {
    expect(stripSoundTags('  hello   world  ')).toBe('hello world')
  })

  it('returns empty for empty input', () => {
    expect(stripSoundTags('')).toBe('')
  })
})

describe('isHallucinationPhrase', () => {
  it('matches canonical Whisper silence phrases', () => {
    expect(isHallucinationPhrase('Thanks for watching')).toBe(true)
    expect(isHallucinationPhrase('Thank you for watching.')).toBe(true)
    expect(isHallucinationPhrase('Subscribe to my channel!')).toBe(true)
    expect(isHallucinationPhrase('Bye.')).toBe(true)
  })

  it('matches case-insensitively and ignores punctuation', () => {
    expect(isHallucinationPhrase('BEEPS')).toBe(true)
    expect(isHallucinationPhrase('music...')).toBe(true)
  })

  it('does not match real sentences', () => {
    expect(isHallucinationPhrase('Open the github tab')).toBe(false)
    expect(isHallucinationPhrase('How are you')).toBe(false)
  })

  it('returns false for empty input', () => {
    expect(isHallucinationPhrase('')).toBe(false)
    expect(isHallucinationPhrase('   ')).toBe(false)
  })
})

describe('sanitize', () => {
  it('cleans sound tags out of a real sentence and sends it', () => {
    const r = sanitize(
      'Explain to me the concept of photosynthesis with the scientific method (beeps) (computerized voice).',
    )
    expect(r.action).toBe('send')
    if (r.action === 'send') {
      expect(r.text).toBe(
        'Explain to me the concept of photosynthesis with the scientific method .',
      )
    }
  })

  it('drops pure sound-tag input', () => {
    // After stripping the tag, nothing is left, so the empty-string
    // branch fires before the phrase-match check. Either reason is
    // fine for the caller; the important thing is the turn is dropped.
    const r = sanitize('(beatbox sounds)')
    expect(r.action).toBe('drop')
  })

  it('drops empty input', () => {
    expect(sanitize('').action).toBe('drop')
    expect(sanitize('   ').action).toBe('drop')
  })

  it('drops canonical silence-hallucination phrases', () => {
    const r = sanitize('Thanks for watching')
    expect(r.action).toBe('drop')
    if (r.action === 'drop') expect(r.reason).toBe('hallucination_only')
  })

  it('drops single-word noise like a sneeze transcribed as a non-command word', () => {
    const r = sanitize('huh')
    expect(r.action).toBe('drop')
    if (r.action === 'drop') expect(r.reason).toBe('short_noise')
  })

  it('keeps single-word legit commands', () => {
    for (const cmd of ['yes', 'no', 'stop', 'cancel', 'continue', 'okay']) {
      const r = sanitize(cmd)
      expect(r.action).toBe('send')
    }
  })

  it('keeps single-word legit commands with trailing punctuation', () => {
    const r = sanitize('Stop.')
    expect(r.action).toBe('send')
  })

  it('drops on low confidence when avgLogprob below threshold', () => {
    const r = sanitize('open the github tab', { avgLogprob: -1.5 })
    expect(r.action).toBe('drop')
    if (r.action === 'drop') expect(r.reason).toBe('low_confidence')
  })

  it('keeps when avgLogprob is above threshold', () => {
    const r = sanitize('open the github tab', { avgLogprob: -0.3 })
    expect(r.action).toBe('send')
  })

  it('ignores avgLogprob when not provided', () => {
    expect(sanitize('open the github tab').action).toBe('send')
  })
})
