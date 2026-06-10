/**
 * Eval-specific constants shared across agents, runners, and capture modules.
 */

export const DEFAULT_TIMEOUT_MS = 30 * 60 * 1000 // 30 minutes
export const MAX_ACTIONS_PER_DELEGATION = 15
// Cold start can take ~5 minutes per Clado; 6 minutes leaves headroom.
export const CLADO_REQUEST_TIMEOUT_MS = 360_000
