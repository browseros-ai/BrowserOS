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
 * ids fall back to a generic mark + a muted tile.
 *
 * `tint` is a pair of tailwind classes (background + foreground) used
 * to give each row a small splash of identity without making the list
 * look like a Christmas tree.
 */
export interface AgentPresentation {
  label: string
  installUrl: string
  mark: FC<SVGProps<SVGSVGElement>>
  tint: string
}

export const AGENT_PRESENTATION: Record<string, AgentPresentation> = {
  'claude-code': {
    label: 'Claude Code',
    installUrl: 'https://claude.ai/code',
    mark: ClaudeMark,
    tint: 'bg-amber-100 text-amber-700 dark:bg-amber-500/15 dark:text-amber-300',
  },
  'claude-desktop': {
    label: 'Claude Desktop',
    installUrl: 'https://claude.ai/download',
    mark: ClaudeDesktopMark,
    tint: 'bg-amber-100 text-amber-700 dark:bg-amber-500/15 dark:text-amber-300',
  },
  cursor: {
    label: 'Cursor',
    installUrl: 'https://cursor.com',
    mark: CursorMark,
    tint: 'bg-sky-100 text-sky-700 dark:bg-sky-500/15 dark:text-sky-300',
  },
  vscode: {
    label: 'VS Code',
    installUrl: 'https://code.visualstudio.com',
    mark: VSCodeMark,
    tint: 'bg-blue-100 text-blue-700 dark:bg-blue-500/15 dark:text-blue-300',
  },
  codex: {
    label: 'Codex',
    installUrl: 'https://github.com/openai/codex',
    mark: CodexMark,
    tint: 'bg-emerald-100 text-emerald-700 dark:bg-emerald-500/15 dark:text-emerald-300',
  },
  zed: {
    label: 'Zed',
    installUrl: 'https://zed.dev',
    mark: ZedMark,
    tint: 'bg-violet-100 text-violet-700 dark:bg-violet-500/15 dark:text-violet-300',
  },
}

const FALLBACK_PRESENTATION: AgentPresentation = {
  label: 'Unknown agent',
  installUrl: '',
  mark: GenericAgentMark,
  tint: 'bg-muted text-muted-foreground',
}

export function presentationFor(id: string): AgentPresentation {
  return AGENT_PRESENTATION[id] ?? { ...FALLBACK_PRESENTATION, label: id }
}

/**
 * Collapse `/Users/<name>/` → `~/` and shorten paths under macOS's
 * Library so config locations stay readable in tight rows. Pure helper
 * with no React deps so it's safe to unit test in isolation.
 */
export function prettifyConfigPath(path: string | null): string | null {
  if (!path) return null
  return path
    .replace(/^\/Users\/[^/]+/, '~')
    .replace(/^\/home\/[^/]+/, '~')
    .replace('Library/Application Support', 'Library/AppSupport')
}
