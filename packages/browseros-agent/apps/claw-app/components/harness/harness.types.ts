export const HARNESSES = [
  'Claude Code',
  'Claude Desktop',
  'Cursor',
  'VS Code',
  'Zed',
  'Codex',
  'Gemini CLI',
  'Hermes',
  'OpenClaw',
] as const

export type Harness = (typeof HARNESSES)[number]

export const RETIRED_HARNESSES = [
  'Claude Desktop',
] as const satisfies readonly Harness[]

/**
 * Harnesses the /mcp screen filters out of the Connected agents list.
 * Retired + BrowserOS-internal harnesses that a user never intentionally
 * installs. Shared with the cockpit onboarding detector so `MCP installed`
 * only lights up for a harness the reader could plausibly have connected
 * from the /mcp screen.
 */
export const HIDDEN_HARNESSES: readonly Harness[] = [
  ...RETIRED_HARNESSES,
  'Hermes',
  'OpenClaw',
  'Gemini CLI',
]

/** True when the harness appears in the /mcp Connected agents list. */
export function isUserFacingHarness(h: Harness): boolean {
  return !HIDDEN_HARNESSES.includes(h)
}
