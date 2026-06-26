import type { FC, SVGProps } from 'react'
import {
  ClaudeDesktopMark,
  ClaudeMark,
  CodexMark,
  CursorMark,
  GenericAgentMark,
  VSCodeMark,
  ZedMark,
} from './agent-marks'

/**
 * Map from agent-mcp-manager AgentId → presentation data the UI uses.
 * Adding a new agent upstream means adding a row here. Unknown agent
 * ids fall back to a generic mark + the same neutral tile.
 *
 * No per-agent tints: marks are SVGL brand artwork that ships with
 * its own colours, so every tile is the same flat white chip and
 * the brand carries the identity.
 */
export interface AgentPresentation {
  label: string
  installUrl: string
  mark: FC<SVGProps<SVGSVGElement>>
}

const AGENT_PRESENTATION: Record<string, AgentPresentation> = {
  'claude-code': {
    label: 'Claude Code',
    installUrl: 'https://claude.ai/code',
    mark: ClaudeMark,
  },
  // Hidden from fresh users in `listAgents` because the integration
  // needs Node on the user's machine. Preserved here so legacy
  // installs that still have a BrowserOS link to Claude Desktop
  // render with the right label and brand mark while the user
  // disconnects.
  'claude-desktop': {
    label: 'Claude Desktop',
    installUrl: 'https://claude.ai/download',
    mark: ClaudeDesktopMark,
  },
  cursor: {
    label: 'Cursor',
    installUrl: 'https://cursor.com',
    mark: CursorMark,
  },
  vscode: {
    label: 'VS Code',
    installUrl: 'https://code.visualstudio.com',
    mark: VSCodeMark,
  },
  codex: {
    label: 'Codex',
    installUrl: 'https://github.com/openai/codex',
    mark: CodexMark,
  },
  zed: {
    label: 'Zed',
    installUrl: 'https://zed.dev',
    mark: ZedMark,
  },
}

const FALLBACK_PRESENTATION: AgentPresentation = {
  label: 'Unknown agent',
  installUrl: '',
  mark: GenericAgentMark,
}

export function presentationFor(id: string): AgentPresentation {
  return AGENT_PRESENTATION[id] ?? { ...FALLBACK_PRESENTATION, label: id }
}
