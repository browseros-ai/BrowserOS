/**
 * Curated starting points for the "Popular agents" helper in the custom-agent
 * dialog. Each command is a starting point; users confirm the exact command and
 * login in the agent's own ACP docs. `mark` is a placeholder for a real brand
 * logo (sourced from svgl.app during a follow-up).
 */
export interface PopularAcpAgent {
  id: string
  label: string
  /** Emoji or letter placeholder for a brand logo. */
  mark: string
  blurb: string
  /** Full launch command that speaks ACP over stdio; omit for docs-only entries. */
  suggestedCommand?: string
  docsUrl: string
}

export const POPULAR_ACP_AGENTS: PopularAcpAgent[] = [
  {
    id: 'opencode',
    label: 'opencode',
    mark: 'oc',
    blurb: 'SST · open-source coding agent (native ACP)',
    suggestedCommand: 'opencode acp',
    docsUrl: 'https://opencode.ai/docs/acp/',
  },
  {
    id: 'hermes',
    label: 'Hermes',
    mark: 'H',
    blurb: 'Nous Research · autonomous agent (native ACP)',
    suggestedCommand: 'hermes acp',
    docsUrl:
      'https://hermes-agent.nousresearch.com/docs/user-guide/features/acp',
  },
  {
    id: 'openclaw',
    label: 'OpenClaw',
    mark: '🐾',
    blurb: 'OpenClaw · ACP orchestrator with a bridge mode',
    suggestedCommand: 'openclaw acp',
    docsUrl: 'https://docs.openclaw.ai/tools/acp-agents',
  },
  {
    id: 'pi',
    label: 'pi',
    mark: 'π',
    blurb: 'Pi Coding Agent · via the pi-acp adapter',
    suggestedCommand: 'npx pi-acp',
    docsUrl: 'https://pi.dev/docs/latest',
  },
  {
    // Antigravity has no first-party ACP command yet (only community adapters),
    // so this is a docs-only entry: no suggested command to copy.
    id: 'antigravity',
    label: 'Antigravity',
    mark: 'A',
    blurb: 'Google · agentic IDE (ACP via community adapters)',
    docsUrl: 'https://antigravity.google',
  },
]
