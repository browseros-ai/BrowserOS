/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Centralized limits and thresholds.
 */

export const AGENT_LIMITS = {
  MAX_TURNS: 100,
  DEFAULT_CONTEXT_WINDOW: 200_000,

  // Compression settings for context compaction heuristics
  COMPRESSION_MIN_HEADROOM: 10_000,
  COMPRESSION_MAX_RATIO: 0.75,
  COMPRESSION_MIN_RATIO: 0.4,

  // Compaction — adaptive trigger
  // Large models (>32K): fixed 20K reserve, rest is trigger budget.
  // Small models (≤32K): 50% reserve instead, so trigger isn't starved.
  COMPACTION_RESERVE_TOKENS: 20_000,

  // Compaction — adaptive keep-recent
  COMPACTION_MAX_KEEP_RECENT: 20_000,
  COMPACTION_KEEP_RECENT_FRACTION: 0.35,

  // Models at or below this use proportional (50%) reserve.
  // Must be ≥ 2 × COMPACTION_FIXED_OVERHEAD (currently 24K) so that
  // the 50% trigger threshold always exceeds the overhead estimate.
  // Below 24K, the overhead cap in computeConfig() handles it.
  COMPACTION_SMALL_CONTEXT_WINDOW: 32_000,

  COMPACTION_MIN_SUMMARIZABLE_INPUT: 4_000,
  COMPACTION_MIN_SUMMARIZABLE_INPUT_SMALL: 1_000,

  // Compaction — summarization
  COMPACTION_MIN_TOKEN_FLOOR: 256,
  COMPACTION_TURN_PREFIX_OUTPUT_RATIO: 0.5,
  COMPACTION_MAX_SUMMARIZATION_INPUT: 100_000,
  COMPACTION_SUMMARIZATION_TIMEOUT_MS: 60_000,
  COMPACTION_SUMMARIZER_OUTPUT_RATIO: 0.8,

  // Compaction — estimation (step 0 / no real usage)
  // Covers system prompt (~2.5K tokens) + tool definitions (~8-9K tokens).
  // computeConfig() caps this at 40% of context window for small models
  // so it never exceeds the trigger threshold on its own.
  COMPACTION_FIXED_OVERHEAD: 12_000,
  COMPACTION_SAFETY_MULTIPLIER: 1.3,
  COMPACTION_IMAGE_TOKEN_ESTIMATE: 1_000,

  // Compaction — pruning (before LLM summarization)
  COMPACTION_PRUNE_KEEP_RECENT_MESSAGES: 6,
  COMPACTION_CLEAR_OUTPUT_MIN_CHARS: 100,

  // Compaction — tool output truncation
  COMPACTION_TOOL_OUTPUT_MAX_CHARS: 15_000,
  COMPACTION_TRANSCRIPT_TOOL_OUTPUT_MAX_CHARS: 2_000,
} as const

export const TOOL_LIMITS = {
  INLINE_PAGE_CONTENT_MAX_CHARS: 5_000,
  FILESYSTEM_READ_MAX_LINES: 500,
  FILESYSTEM_READ_MAX_CHARS: 15_000,
} as const

export const PAGINATION = {
  DEFAULT_PAGE_SIZE: 20,
} as const

export const CDP_LIMITS = {
  CONNECT_MAX_RETRIES: 3,
  RECONNECT_MAX_RETRIES: 3,
} as const

export const CONTENT_LIMITS = {
  BODY_CONTEXT_SIZE: 10_000,
  MAX_QUEUE_SIZE: 1_000,
  CONSOLE_META_CHAR: 1_000,
  CONSOLE_BUFFER_MAX_ENTRIES: 500,
  CONSOLE_DEFAULT_LIMIT: 50,
  CONSOLE_MAX_LIMIT: 200,
} as const

export const REFERRAL_LIMITS = {
  MAX_DAILY_CREDITS: 500,
  CREDITS_PER_REFERRAL: 200,
} as const
