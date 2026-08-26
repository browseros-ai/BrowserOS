/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Centralized limits and thresholds.
 */

export const RECORDING_INGEST_MAX_BYTES = 16 * 1024 * 1024
/** Aggregate target used until a server explicitly advertises a larger ceiling. */
export const RECORDING_INGEST_FALLBACK_MAX_BYTES = 2 * 1024 * 1024

export const AGENT_LIMITS = {
  MAX_TURNS: 100,
  DEFAULT_CONTEXT_WINDOW: 200_000,

  // Compaction trigger. Reserve is headroom for the model's own response and is
  // capped at half the context window, so small models keep a usable budget.
  COMPACTION_RESERVE_TOKENS: 20_000,

  // Compaction estimation. `prepareStep` cannot see the system prompt (~2.5K)
  // or the tool schemas (~8-9K), so they are added as a flat overhead.
  COMPACTION_FIXED_OVERHEAD: 12_000,
  // Images are counted at a flat render cost, never by their encoded length.
  COMPACTION_IMAGE_TOKEN_ESTIMATE: 1_000,

  // Compaction pruning. Tool calls older than this many messages are cleared.
  COMPACTION_PRUNE_KEEP_RECENT_MESSAGES: 6,
} as const

export const TOOL_LIMITS = {
  INLINE_PAGE_CONTENT_MAX_CHARS: 5_000,
  GREP_MAX_MATCHES: 200,
  GREP_MATCH_LINE_MAX_CHARS: 500,
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
} as const

export const AGENT_HARNESS_LIMITS = {
  AGENT_NAME_MAX_CHARS: 80,
  /** Maximum number of messages allowed in an agent's pending queue. */
  QUEUE_MAX_LENGTH: 50,
  /** Maximum size in bytes for a single queued message's text. */
  QUEUE_MESSAGE_MAX_BYTES: 64 * 1024,
} as const
