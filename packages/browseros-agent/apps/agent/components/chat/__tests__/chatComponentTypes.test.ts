import { describe, expect, it } from 'bun:test'
import {
  formatContextSize,
  formatModelShortName,
  type Provider,
} from '../chatComponentTypes'

// ═══════════════════════════════════════════════════════════════════════════
// formatContextSize — edge cases
// ═══════════════════════════════════════════════════════════════════════════

describe('formatContextSize', () => {
  it('formats 128000 → "128K"', () => {
    expect(formatContextSize(128_000)).toBe('128K')
  })

  it('formats 200000 → "200K"', () => {
    expect(formatContextSize(200_000)).toBe('200K')
  })

  it('formats 1000000 → "1M"', () => {
    expect(formatContextSize(1_000_000)).toBe('1M')
  })

  it('formats 2000000 → "2M"', () => {
    expect(formatContextSize(2_000_000)).toBe('2M')
  })

  it('formats 1500000 → "1.5M"', () => {
    expect(formatContextSize(1_500_000)).toBe('1.5M')
  })

  // ── Edge cases ────────────────────────────────────────────────────────

  it('returns empty string for undefined', () => {
    expect(formatContextSize(undefined)).toBe('')
  })

  it('returns empty string for 0', () => {
    expect(formatContextSize(0)).toBe('')
  })

  it('returns empty string for negative values', () => {
    expect(formatContextSize(-1)).toBe('')
    expect(formatContextSize(-128000)).toBe('')
  })

  it('returns empty string for NaN', () => {
    expect(formatContextSize(Number.NaN)).toBe('')
  })

  it('formats small values under 1000 correctly (e.g. 500 → "0.5K")', () => {
    expect(formatContextSize(500)).toBe('0.5K')
  })

  it('formats 1000 → "1K"', () => {
    expect(formatContextSize(1_000)).toBe('1K')
  })

  it('formats 400000 → "400K"', () => {
    expect(formatContextSize(400_000)).toBe('400K')
  })

  it('formats 999999 → "1000K" (boundary: should not overflow to M)', () => {
    // 999999 / 1000 = 999.999 → rounds to 1000 → "1000K"
    expect(formatContextSize(999_999)).toBe('1000K')
  })
})

// ═══════════════════════════════════════════════════════════════════════════
// formatModelShortName — edge cases
// ═══════════════════════════════════════════════════════════════════════════

describe('formatModelShortName', () => {
  it('returns empty string for undefined', () => {
    expect(formatModelShortName(undefined)).toBe('')
  })

  it('returns empty string for empty string', () => {
    expect(formatModelShortName('')).toBe('')
  })

  // ── Known models ─────────────────────────────────────────────────────

  it('strips 8-digit date suffix from claude models, then converts version', () => {
    // claude-sonnet-4-6-20250514 → strip date → claude-sonnet-4-6
    // Then regex: .+="claude-sonnet", then "4-6" → claude-sonnet-4.6
    expect(formatModelShortName('claude-sonnet-4-6-20250514')).toBe(
      'claude-sonnet-4.6',
    )
  })

  it('handles claude-sonnet-4-6 without date suffix, converts version', () => {
    expect(formatModelShortName('claude-sonnet-4-6')).toBe('claude-sonnet-4.6')
  })

  it('handles gpt-4o', () => {
    expect(formatModelShortName('gpt-4o')).toBe('gpt-4o')
  })

  it('handles gpt-5.4', () => {
    expect(formatModelShortName('gpt-5.4')).toBe('gpt-5.4')
  })

  it('converts claude-sonnet-4-6 version pattern → claude-sonnet-4.6', () => {
    // "claude-sonnet-4-6" matches .+="claude-sonnet", 4, 6 → claude-sonnet-4.6
    expect(formatModelShortName('claude-sonnet-4-6')).toBe('claude-sonnet-4.6')
  })

  it('keeps o3-mini as-is (no prefix stripping)', () => {
    // "o3-mini" is already short and meaningful
    expect(formatModelShortName('o3-mini')).toBe('o3-mini')
  })

  it('keeps o4-mini as-is', () => {
    expect(formatModelShortName('o4-mini')).toBe('o4-mini')
  })

  it('keeps deepseek-chat as-is', () => {
    // "deepseek-chat" is meaningful — don't strip to "chat"
    expect(formatModelShortName('deepseek-chat')).toBe('deepseek-chat')
  })

  it('handles qwen3-coder-plus', () => {
    // qwen- prefix match won't match "qwen3-" — "qwen-" != "qwen3-"
    expect(formatModelShortName('qwen3-coder-plus')).toBe('qwen3-coder-plus')
  })

  it('converts trailing version segments: gpt-4-1 → gpt-4.1', () => {
    // Last two segments are numeric → convert hyphen to dot
    expect(formatModelShortName('gpt-4-1')).toBe('gpt-4.1')
  })

  it('converts claude-sonnet-4-6 date-stripped → claude-sonnet-4.6', () => {
    // After date strip: "claude-sonnet-4-6" → regex matches "sonnet-4-6" no,
    // actually "claude-sonnet-4-6" → .+="claude-sonnet" then "4-6" → "claude-sonnet-4.6"
    expect(formatModelShortName('claude-sonnet-4-6-20250514')).toBe(
      'claude-sonnet-4.6',
    )
  })

  // ── Truncation ───────────────────────────────────────────────────────

  it('truncates very long model names to 24 chars', () => {
    const longName =
      'this-is-an-extremely-long-model-name-that-should-be-truncated-for-display'
    const result = formatModelShortName(longName)
    expect(result.length).toBeLessThanOrEqual(24)
    expect(result).toMatch(/\.\.\.$/)
  })

  // ── browseros-auto ───────────────────────────────────────────────────

  it('handles browseros-auto as-is (no prefix match)', () => {
    expect(formatModelShortName('browseros-auto')).toBe('browseros-auto')
  })

  // ── Edge: prefix removal produces empty string ───────────────────────

  it('handles "claude-" edge case (prefix-only, nothing after)', () => {
    // No date suffix, no version conversion — stays as-is
    expect(formatModelShortName('claude-')).toBe('claude-')
  })
})

// ═══════════════════════════════════════════════════════════════════════════
// Provider type — extended fields modelId + contextWindow
// ═══════════════════════════════════════════════════════════════════════════

describe('Provider type', () => {
  it('accepts modelId and contextWindow as optional fields', () => {
    const provider: Provider = {
      id: 'test',
      name: 'Test Provider',
      type: 'openai',
      kind: 'llm',
      modelId: 'gpt-4o',
      contextWindow: 128000,
    }
    expect(provider.modelId).toBe('gpt-4o')
    expect(provider.contextWindow).toBe(128000)
  })

  it('accepts Provider without modelId or contextWindow', () => {
    const provider: Provider = {
      id: 'test',
      name: 'Test Provider',
      type: 'openai',
      kind: 'llm',
    }
    expect(provider.modelId).toBeUndefined()
    expect(provider.contextWindow).toBeUndefined()
  })

  it('ACP Provider can have modelId without contextWindow', () => {
    const provider: Provider = {
      id: 'agent-1',
      name: 'Review Bot',
      type: 'acp',
      kind: 'acp',
      modelId: 'sonnet',
      // contextWindow intentionally omitted for ACP
    }
    expect(provider.modelId).toBe('sonnet')
    expect(provider.contextWindow).toBeUndefined()
  })
})
