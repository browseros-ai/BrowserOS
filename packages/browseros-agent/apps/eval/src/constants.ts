/**
 * Eval-specific constants shared across agents, runners, and capture modules.
 */

export const DEFAULT_TIMEOUT_MS = 30 * 60 * 1000 // 30 minutes
export const SCREENSHOT_TIMEOUT_MS = 65_000 // 65s — ensures we get extension's error (60s)
export const MAX_ACTIONS_PER_DELEGATION = 15
export const CLADO_REQUEST_TIMEOUT_MS = 120_000
