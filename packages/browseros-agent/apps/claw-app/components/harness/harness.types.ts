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

/** The known harness for a session's harness name, if it is one we draw. */
export function asHarness(name: string | undefined): Harness | undefined {
  return HARNESSES.find((harness) => harness === name)
}

// The server names a session's agent by a short client prefix ("claude",
// "codex") when it cannot tell the exact app; these are the ones we can draw.
const AGENT_PREFIXES: Record<string, { harness: Harness; name: string }> = {
  claude: { harness: 'Claude Code', name: 'Claude' },
  'claude-code': { harness: 'Claude Code', name: 'Claude Code' },
  codex: { harness: 'Codex', name: 'Codex' },
  cursor: { harness: 'Cursor', name: 'Cursor' },
  opencode: { harness: 'OpenCode', name: 'OpenCode' },
  antigravity: { harness: 'Antigravity', name: 'Antigravity' },
  vscode: { harness: 'VS Code', name: 'VS Code' },
  zed: { harness: 'Zed', name: 'Zed' },
}

export interface AgentIdentity {
  /** The harness whose mark to draw, when known. */
  harness?: Harness
  /** The agent's name as a person would say it. */
  name: string
}

/** Who ran a session, from its harness name or else its label or slug. */
export function agentIdentity(
  harness: string | undefined,
  label: string | undefined,
): AgentIdentity {
  const exact = asHarness(harness)
  if (exact) return { harness: exact, name: exact }
  const raw = (label ?? '').trim()
  const known = AGENT_PREFIXES[raw.toLowerCase()]
  if (known) return known
  const named = asHarness(raw)
  if (named) return { harness: named, name: named }
  return { name: raw || 'Your AI agent' }
}
