/**
 * @license AGPL-3.0-or-later
 * Copyright 2026 TRIOS
 *
 * TRIOS Absorb Smart — Types
 * Smart file sorting into virtual branches.
 */

/** A single file grouped into a branch plan */
export interface PlannedFile {
  path: string
  status: 'added' | 'modified' | 'deleted' | 'renamed' | 'untracked'
  reason: string
}

/** A branch with its planned files */
export interface PlannedBranch {
  branchName: string
  files: PlannedFile[]
  confidence: number // 0-1, how confident the strategy is
}

/** The full absorb plan returned by a strategy */
export interface AbsorbPlan {
  strategy: 'by-directory' | 'by-type' | 'auto'
  branches: PlannedBranch[]
  unassigned: PlannedFile[] // files that didn't fit any group
  summary: string
}

/** Input to the absorb engine */
export interface AbsorbInput {
  /** Current branch name */
  currentBranch: string
  /** All changed files with their paths and statuses */
  changedFiles: Array<{
    path: string
    status: 'added' | 'modified' | 'deleted' | 'renamed' | 'untracked'
  }>
  /** Existing branch names (to avoid conflicts) */
  existingBranches: string[]
}

/** Result of executing an absorb plan */
export interface AbsorbResult {
  ok: boolean
  reason: string
  plan?: AbsorbPlan
  branchesCreated?: string[]
  filesStaged?: number
}
