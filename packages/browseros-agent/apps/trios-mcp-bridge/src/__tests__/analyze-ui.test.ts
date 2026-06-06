/**
 * @license AGPL-3.0-or-later
 * Copyright 2026 TRIOS
 *
 * Phase 2 Tests: gitbutler_analyze_ui structured output
 *
 * Three mandatory tests:
 * 1. GB with multiple unstaged files
 * 2. GB with only staged files
 * 3. GB clean (no changes)
 *
 * Run: bun test src/__tests__/analyze-ui.test.ts
 */

import { describe, expect, test } from 'bun:test'
import {
  analyzeGitButlerUI,
  type RawStatus,
} from '../vision/gitbutler-analyzer.js'

// ==========================================
// Test 1: Multiple unstaged files
// ==========================================
describe('Test 1: Multiple unstaged files', () => {
  const cliStatus: RawStatus = {
    branch: 'feature/auth',
    ahead: 2,
    behind: 0,
    staged: [],
    unstaged: [
      { path: 'src/auth/login.ts', status: 'modified' },
      { path: 'src/auth/register.ts', status: 'modified' },
      { path: 'src/auth/types.ts', status: 'modified' },
      { path: 'src/utils/helpers.ts', status: 'deleted' },
    ],
    untracked: ['src/auth/new-module.ts'],
    conflicted: [],
  }

  const snapshot = `
[1] GitButler
[2] main toolbar
[3] sidebar
[4] Branch: feature/auth
[5] Stacks
[6] stack: feature/auth (2 commits ahead)
[7] Changed files
[8] modified src/auth/login.ts
[9] modified src/auth/register.ts
[10] modified src/auth/types.ts
[11] deleted src/utils/helpers.ts
[12] untracked src/auth/new-module.ts
`

  test('returns correct activeBranch', () => {
    const result = analyzeGitButlerUI(snapshot, cliStatus)
    expect(result.activeBranch).toBe('feature/auth')
  })

  test('returns all changed files with paths', () => {
    const result = analyzeGitButlerUI(snapshot, cliStatus)
    expect(result.changedFiles.length).toBe(5)

    const paths = result.changedFiles.map((f) => f.path)
    expect(paths).toContain('src/auth/login.ts')
    expect(paths).toContain('src/auth/register.ts')
    expect(paths).toContain('src/auth/types.ts')
    expect(paths).toContain('src/utils/helpers.ts')
    expect(paths).toContain('src/auth/new-module.ts')
  })

  test('all files are unstaged (staged: false)', () => {
    const result = analyzeGitButlerUI(snapshot, cliStatus)
    const stagedFiles = result.changedFiles.filter((f) => f.staged)
    expect(stagedFiles.length).toBe(0)

    const unstagedFiles = result.changedFiles.filter((f) => !f.staged)
    expect(unstagedFiles.length).toBe(5)
  })

  test('file statuses are correct', () => {
    const result = analyzeGitButlerUI(snapshot, cliStatus)
    const byPath = Object.fromEntries(
      result.changedFiles.map((f) => [f.path, f]),
    )

    expect(byPath['src/auth/login.ts'].status).toBe('modified')
    expect(byPath['src/utils/helpers.ts'].status).toBe('deleted')
    expect(byPath['src/auth/new-module.ts'].status).toBe('untracked')
  })

  test('isClean is false', () => {
    const result = analyzeGitButlerUI(snapshot, cliStatus)
    expect(result.isClean).toBe(false)
  })

  test('suggestedActions include staging', () => {
    const result = analyzeGitButlerUI(snapshot, cliStatus)
    expect(result.suggestedActions.length).toBeGreaterThan(0)
    const hasStaging = result.suggestedActions.some((a) =>
      a.toLowerCase().includes('stage'),
    )
    expect(hasStaging).toBe(true)
  })

  test('no null fields', () => {
    const result = analyzeGitButlerUI(snapshot, cliStatus)
    expect(result.activeBranch).not.toBeNull()
    expect(result.changedFiles).not.toBeNull()
    expect(result.stacks).not.toBeNull()
    expect(result.summary).not.toBeNull()
    expect(result.suggestedActions).not.toBeNull()
  })
})

// ==========================================
// Test 2: Only staged files
// ==========================================
describe('Test 2: Only staged files', () => {
  const cliStatus: RawStatus = {
    branch: 'main',
    ahead: 0,
    behind: 1,
    staged: [
      { path: 'src/core/engine.ts', status: 'added' },
      { path: 'src/core/engine.test.ts', status: 'added' },
      { path: 'README.md', status: 'modified' },
    ],
    unstaged: [],
    untracked: [],
    conflicted: [],
  }

  const snapshot = `
[1] GitButler
[4] Branch: main
[5] Staged changes (3 files)
[6] added src/core/engine.ts
[7] added src/core/engine.test.ts
[8] modified README.md
`

  test('returns correct activeBranch', () => {
    const result = analyzeGitButlerUI(snapshot, cliStatus)
    expect(result.activeBranch).toBe('main')
  })

  test('returns all staged files with staged: true', () => {
    const result = analyzeGitButlerUI(snapshot, cliStatus)
    expect(result.changedFiles.length).toBe(3)

    const allStaged = result.changedFiles.every((f) => f.staged)
    expect(allStaged).toBe(true)
  })

  test('file paths are exact', () => {
    const result = analyzeGitButlerUI(snapshot, cliStatus)
    const paths = result.changedFiles.map((f) => f.path)
    expect(paths).toContain('src/core/engine.ts')
    expect(paths).toContain('src/core/engine.test.ts')
    expect(paths).toContain('README.md')
  })

  test('file statuses are correct', () => {
    const result = analyzeGitButlerUI(snapshot, cliStatus)
    const byPath = Object.fromEntries(
      result.changedFiles.map((f) => [f.path, f]),
    )

    expect(byPath['src/core/engine.ts'].status).toBe('added')
    expect(byPath['src/core/engine.test.ts'].status).toBe('added')
    expect(byPath['README.md'].status).toBe('modified')
  })

  test('isClean is false', () => {
    const result = analyzeGitButlerUI(snapshot, cliStatus)
    expect(result.isClean).toBe(false)
  })

  test('suggestedActions include commit', () => {
    const result = analyzeGitButlerUI(snapshot, cliStatus)
    const hasCommit = result.suggestedActions.some((a) =>
      a.toLowerCase().includes('commit'),
    )
    expect(hasCommit).toBe(true)
  })

  test('no null fields', () => {
    const result = analyzeGitButlerUI(snapshot, cliStatus)
    expect(result.activeBranch).not.toBeNull()
    expect(result.changedFiles).not.toBeNull()
    expect(result.stacks).not.toBeNull()
    expect(result.summary).not.toBeNull()
    expect(result.suggestedActions).not.toBeNull()
  })
})

// ==========================================
// Test 3: Clean workspace
// ==========================================
describe('Test 3: Clean workspace (no changes)', () => {
  const cliStatus: RawStatus = {
    branch: 'main',
    ahead: 0,
    behind: 0,
    staged: [],
    unstaged: [],
    untracked: [],
    conflicted: [],
  }

  const snapshot = `
[1] GitButler
[4] Branch: main
[5] No changes
[6] Your workspace is clean
`

  test('returns correct activeBranch', () => {
    const result = analyzeGitButlerUI(snapshot, cliStatus)
    expect(result.activeBranch).toBe('main')
  })

  test('changedFiles is empty', () => {
    const result = analyzeGitButlerUI(snapshot, cliStatus)
    expect(result.changedFiles.length).toBe(0)
  })

  test('isClean is true', () => {
    const result = analyzeGitButlerUI(snapshot, cliStatus)
    expect(result.isClean).toBe(true)
  })

  test('summary mentions clean', () => {
    const result = analyzeGitButlerUI(snapshot, cliStatus)
    expect(result.summary.toLowerCase()).toContain('clean')
  })

  test('suggestedActions indicate no actions needed', () => {
    const result = analyzeGitButlerUI(snapshot, cliStatus)
    const hasClean = result.suggestedActions.some(
      (a) =>
        a.toLowerCase().includes('clean') ||
        a.toLowerCase().includes('no action'),
    )
    expect(hasClean).toBe(true)
  })

  test('no null fields', () => {
    const result = analyzeGitButlerUI(snapshot, cliStatus)
    expect(result.activeBranch).not.toBeNull()
    expect(result.changedFiles).not.toBeNull()
    expect(result.stacks).not.toBeNull()
    expect(result.summary).not.toBeNull()
    expect(result.suggestedActions).not.toBeNull()
  })
})

// ==========================================
// Edge case: No CLI status, snapshot only
// ==========================================
describe('Edge case: No CLI status (snapshot only)', () => {
  const snapshot = `
[1] GitButler
[4] Branch: feature/vision
[7] modified src/vision/analyzer.ts
[8] added src/vision/types.ts
`

  test('extracts branch from snapshot', () => {
    const result = analyzeGitButlerUI(snapshot, null)
    expect(result.activeBranch).toBe('feature/vision')
  })

  test('extracts files from snapshot', () => {
    const result = analyzeGitButlerUI(snapshot, null)
    expect(result.changedFiles.length).toBeGreaterThan(0)
  })

  test('staged is false (cannot determine from snapshot)', () => {
    const result = analyzeGitButlerUI(snapshot, null)
    const allUnstaged = result.changedFiles.every((f) => !f.staged)
    expect(allUnstaged).toBe(true)
  })

  test('no null fields', () => {
    const result = analyzeGitButlerUI(snapshot, null)
    expect(result.activeBranch).not.toBeNull()
    expect(result.changedFiles).not.toBeNull()
    expect(result.stacks).not.toBeNull()
    expect(result.summary).not.toBeNull()
    expect(result.suggestedActions).not.toBeNull()
  })
})

// ==========================================
// Edge case: No data at all
// ==========================================
describe('Edge case: No data', () => {
  test('handles null snapshot and null status', () => {
    const result = analyzeGitButlerUI(null, null)
    expect(result.activeBranch).toBe('unknown')
    expect(result.changedFiles).toEqual([])
    expect(result.isClean).toBe(true)
    expect(result.suggestedActions.length).toBeGreaterThan(0)
  })
})
