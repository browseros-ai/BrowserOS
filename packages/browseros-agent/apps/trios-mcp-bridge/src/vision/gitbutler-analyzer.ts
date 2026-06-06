/**
 * @license AGPL-3.0-or-later
 * Copyright 2026 TRIOS
 *
 * GitButler UI Analyzer
 * Parses accessibility tree snapshots + CLI status to extract structured data.
 */

export interface AnalyzedUI {
  /** Active branch name (from CLI — ground truth) */
  activeBranch: string
  /** List of changed files with full paths and staging status */
  changedFiles: AnalyzedFile[]
  /** Stack/branch names detected */
  stacks: string[]
  /** Whether the workspace is clean */
  isClean: boolean
  /** Human-readable summary */
  summary: string
  /** Suggested next actions */
  suggestedActions: string[]
}

export interface AnalyzedFile {
  path: string
  status: 'added' | 'modified' | 'deleted' | 'renamed' | 'untracked'
  staged: boolean
  oldPath?: string
}

export interface RawStatus {
  branch: string
  ahead: number
  behind: number
  staged: Array<{ path: string; status: string; oldPath?: string }>
  unstaged: Array<{ path: string; status: string; oldPath?: string }>
  untracked: string[]
  conflicted: string[]
}

/**
 * Analyze GitButler UI from accessibility tree + CLI status.
 *
 * This is the core analysis function. It combines:
 * 1. CLI status (ground truth for file paths and staging)
 * 2. Accessibility tree (for UI-specific elements like stacks, panels)
 *
 * Returns structured JSON with no null fields.
 */
export function analyzeGitButlerUI(
  snapshot: string | null,
  cliStatus: RawStatus | null,
): AnalyzedUI {
  // If we have CLI status, use it as ground truth
  if (cliStatus) {
    return analyzeFromCliStatus(snapshot, cliStatus)
  }

  // Fallback: parse only from accessibility tree
  return analyzeFromSnapshotOnly(snapshot)
}

/**
 * Primary path: CLI status (ground truth) + snapshot (UI context)
 */
function analyzeFromCliStatus(
  snapshot: string | null,
  status: RawStatus,
): AnalyzedUI {
  const changedFiles: AnalyzedFile[] = []

  // Staged files
  for (const f of status.staged) {
    changedFiles.push({
      path: f.path,
      status: normalizeStatus(f.status),
      staged: true,
      oldPath: f.oldPath,
    })
  }

  // Unstaged files
  for (const f of status.unstaged) {
    changedFiles.push({
      path: f.path,
      status: normalizeStatus(f.status),
      staged: false,
      oldPath: f.oldPath,
    })
  }

  // Untracked files
  for (const f of status.untracked) {
    changedFiles.push({
      path: f,
      status: 'untracked',
      staged: false,
    })
  }

  // Extract stacks from snapshot if available
  const stacks = snapshot ? extractStacksFromSnapshot(snapshot) : []

  const isClean = changedFiles.length === 0 && status.conflicted.length === 0

  const stagedCount = changedFiles.filter((f) => f.staged).length
  const unstagedCount = changedFiles.filter((f) => !f.staged).length

  const summary = isClean
    ? `Workspace is clean. Branch: ${status.branch} (↑${status.ahead} ↓${status.behind})`
    : `Branch: ${status.branch} (↑${status.ahead} ↓${status.behind}). ${stagedCount} staged, ${unstagedCount} unstaged, ${status.untracked.length} untracked, ${status.conflicted.length} conflicted files.`

  const suggestedActions = generateSuggestions(status, changedFiles)

  return {
    activeBranch: status.branch,
    changedFiles,
    stacks,
    isClean,
    summary,
    suggestedActions,
  }
}

/**
 * Fallback: parse from accessibility tree only (less accurate)
 */
function analyzeFromSnapshotOnly(snapshot: string | null): AnalyzedUI {
  if (!snapshot) {
    return {
      activeBranch: 'unknown',
      changedFiles: [],
      stacks: [],
      isClean: true,
      summary: 'No data available — both CLI status and snapshot are empty.',
      suggestedActions: [
        'Ensure GitButler is open in the browser',
        'Check that the working directory contains a git repository',
      ],
    }
  }

  // Try to extract branch name from snapshot
  const branchMatch = snapshot.match(/(?:branch|ref|head)[:\s]+([^\n\][,]+)/i)
  const activeBranch = branchMatch ? branchMatch[1].trim() : 'unknown'

  // Try to extract file paths from snapshot
  const changedFiles: AnalyzedFile[] = []
  const filePatterns = [
    // Match common source file extensions
    /(?:modified|added|deleted|renamed|changed)[:\s]+([^\s,]+\.(ts|tsx|js|jsx|py|rs|go|md|css|html|json|yaml|yml|toml|sql|sh|txt|xml|svg|lock))/gi,
    // Match file paths in brackets
    /\[(\d+)\]\s+(.+?\.(ts|tsx|js|jsx|py|rs|go|md|css|html|json|yaml|yml|toml))/gi,
    // Match generic file-like paths
    /[\s"]([a-zA-Z0-9_./-]+\.[a-zA-Z]{1,10})[\s"]/g,
  ]

  const seenPaths = new Set<string>()
  for (const pattern of filePatterns) {
    const matches = [...snapshot.matchAll(pattern)]
    for (const match of matches) {
      const filePath = match[1]
      if (
        filePath &&
        !seenPaths.has(filePath) &&
        !filePath.includes('node_modules')
      ) {
        seenPaths.add(filePath)
        changedFiles.push({
          path: filePath,
          status: 'modified', // Default assumption
          staged: false, // Can't determine from snapshot alone
        })
      }
    }
  }

  const stacks = extractStacksFromSnapshot(snapshot)
  const isClean = changedFiles.length === 0

  return {
    activeBranch,
    changedFiles,
    stacks,
    isClean,
    summary: isClean
      ? `Branch: ${activeBranch}. No changed files detected in snapshot.`
      : `Branch: ${activeBranch}. ${changedFiles.length} files detected in snapshot (staging status unknown — use CLI for accuracy).`,
    suggestedActions: isClean
      ? ['Workspace appears clean. Make some changes to test.']
      : [
          'Use gitbutler_workspace_status for accurate staging information',
          'Use gitbutler_commit_visible to commit changes',
        ],
  }
}

/**
 * Extract stack/branch names from accessibility tree
 */
function extractStacksFromSnapshot(snapshot: string): string[] {
  const stacks: string[] = []

  // Look for common GitButler UI patterns for stacks
  const patterns = [
    // "Stack: feature/auth" or "stack: feature-auth"
    /stack[:\s]+([a-zA-Z0-9_\-/]+)/gi,
    // Branch names in tree items
    /treeitem[^>]*>([^<]*(?:feature|fix|hotfix|main|master|develop|release)[^<]*)</gi,
    // Applied branches section
    /applied[:\s]+([a-zA-Z0-9_\-/]+)/gi,
    // Virtual branch names in list items
    /listitem[^>]*>([^<]{2,50})</gi,
  ]

  const seen = new Set<string>()
  for (const pattern of patterns) {
    const matches = [...snapshot.matchAll(pattern)]
    for (const match of matches) {
      const name = match[1].trim()
      if (name && name.length > 1 && name.length < 60 && !seen.has(name)) {
        seen.add(name)
        stacks.push(name)
      }
    }
  }

  return stacks
}

/**
 * Generate suggested actions based on workspace state
 */
function generateSuggestions(
  status: RawStatus,
  changedFiles: AnalyzedFile[],
): string[] {
  const actions: string[] = []

  if (status.conflicted.length > 0) {
    actions.push(
      `⚠️ Resolve ${status.conflicted.length} conflict(s) before proceeding`,
    )
    return actions
  }

  const unstagedFiles = changedFiles.filter((f) => !f.staged)
  const stagedFiles = changedFiles.filter((f) => f.staged)
  const untrackedFiles = changedFiles.filter((f) => f.status === 'untracked')

  if (unstagedFiles.length > 0 && stagedFiles.length === 0) {
    actions.push(
      `gitbutler_stage — stage ${unstagedFiles.length} file(s) before committing`,
    )
  }

  if (stagedFiles.length > 0) {
    actions.push(
      `gitbutler_commit_visible — commit ${stagedFiles.length} staged file(s)`,
    )
  }

  if (untrackedFiles.length > 0) {
    actions.push(
      `${untrackedFiles.length} untracked file(s) — stage them if they should be tracked`,
    )
  }

  if (status.ahead > 0) {
    actions.push(
      `gitbutler_push_stack — push ${status.ahead} commit(s) to remote`,
    )
  }

  if (status.behind > 0) {
    actions.push(`gitbutler_pull — pull ${status.behind} commit(s) from remote`)
  }

  if (actions.length === 0) {
    actions.push('Workspace is clean — no actions needed')
  }

  return actions
}

/**
 * Normalize status string from git/GB to our enum
 */
function normalizeStatus(status: string): AnalyzedFile['status'] {
  const lower = status.toLowerCase()
  if (lower.includes('add') || lower === 'a') return 'added'
  if (lower.includes('delet') || lower === 'd') return 'deleted'
  if (lower.includes('rename') || lower === 'r') return 'renamed'
  if (lower.includes('untrack') || lower === '?') return 'untracked'
  return 'modified'
}
