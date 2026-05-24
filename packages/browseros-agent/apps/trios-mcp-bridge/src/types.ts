/**
 * @license AGPL-3.0-or-later
 * Copyright 2026 TRIOS
 *
 * TRIOS MCP Bridge — Shared types
 */

/** Result from BrowserOS screenshot tool */
export interface ScreenshotResult {
  data: string // base64
  mimeType: string
  devicePixelRatio: number
}

/** Result from BrowserOS snapshot tool */
export interface SnapshotResult {
  snapshot: string
}

/** GitButler workspace status */
export interface GitButlerStatus {
  branch: string
  ahead: number
  behind: number
  staged: FileChange[]
  unstaged: FileChange[]
  untracked: string[]
  conflicted: string[]
}

export interface FileChange {
  path: string
  status: 'added' | 'modified' | 'deleted' | 'renamed'
  oldPath?: string
}

/** Branch info from GitButler */
export interface BranchInfo {
  name: string
  isCurrent: boolean
  isRemote: boolean
  ahead: number
  behind: number
}

/** Commit result */
export interface CommitResult {
  success: boolean
  hash?: string
  error?: string
}

/** Vision analysis result */
export interface VisionAnalysis {
  activeBranch: string | null
  changedFiles: string[]
  stacks: string[]
  summary: string
  suggestedActions: string[]
}

/** Tool context passed to all bridge tools */
export interface BridgeToolContext {
  browserosClient: import('../src/clients/browseros-client').BrowserOSClient
  gitbutlerClient: import('../src/clients/gitbutler-client').GitButlerMcpClient
  config: import('../src/config').BridgeConfig
  ragClient: import('../src/clients/trios-rag-client').TriosRagClient
}
