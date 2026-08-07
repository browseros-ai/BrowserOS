export const HARNESSES = [
  'Claude Code',
  'Codex',
  'Cursor',
  'OpenCode',
  'Antigravity',
  'VS Code',
  'Zed',
] as const

export type Harness = (typeof HARNESSES)[number]

/**
 * Retired harnesses used to appear on the /mcp screen; kept as an
 * empty tuple so the type + filter helpers below stay valid if a
 * future entry needs to be retired without immediate removal.
 */
export const RETIRED_HARNESSES = [] as const satisfies readonly Harness[]

/**
 * Harnesses the /mcp screen filters out of the Connected agents list.
 * Shared with the cockpit onboarding detector so `MCP installed` only
 * lights up for a harness the reader could plausibly have connected
 * from the /mcp screen.
 */
export const HIDDEN_HARNESSES: readonly Harness[] = [...RETIRED_HARNESSES]

/** True when the harness appears in the /mcp Connected agents list. */
export function isUserFacingHarness(h: Harness): boolean {
  return !HIDDEN_HARNESSES.includes(h)
}
