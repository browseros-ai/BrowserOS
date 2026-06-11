/**
 * Map from agent-mcp-manager AgentId -> presentation data the UI uses.
 * Adding a new agent upstream means adding a row here + an entry in
 * ProviderIcon below. Unknown agent ids fall back to a generic look.
 */
export interface AgentPresentation {
  label: string
  installUrl: string
}

export const AGENT_PRESENTATION: Record<string, AgentPresentation> = {
  'claude-code': {
    label: 'Claude Code',
    installUrl: 'https://claude.ai/code',
  },
  'claude-desktop': {
    label: 'Claude Desktop',
    installUrl: 'https://claude.ai/download',
  },
  cursor: {
    label: 'Cursor',
    installUrl: 'https://cursor.com',
  },
  vscode: {
    label: 'VS Code',
    installUrl: 'https://code.visualstudio.com',
  },
  gemini: {
    label: 'Gemini CLI',
    installUrl: 'https://github.com/google-gemini/gemini-cli',
  },
  codex: {
    label: 'Codex',
    installUrl: 'https://github.com/openai/codex',
  },
  zed: {
    label: 'Zed',
    installUrl: 'https://zed.dev',
  },
}

export function presentationFor(id: string): AgentPresentation {
  return AGENT_PRESENTATION[id] ?? { label: id, installUrl: '' }
}
