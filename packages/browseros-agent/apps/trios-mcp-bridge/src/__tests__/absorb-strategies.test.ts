/**
 * @license AGPL-3.0-or-later
 * Copyright 2026 TRIOS
 *
 * Tests for absorb strategies — Issue #6
 *
 * Mandatory scenarios:
 * 1. by-directory: 7 files across 3 dirs → 3 branches
 * 2. by-type: mixed extensions → grouped by category
 * 3. auto: picks best strategy
 * 4. dryRun returns plan without execution
 * 5. Empty workspace → clean plan
 * 6. Single file → single branch
 */

import { describe, expect, test } from 'bun:test'
import {
  runStrategy,
  strategyAuto,
  strategyByDirectory,
  strategyByType,
} from '../absorb/strategies.js'
import type { AbsorbInput } from '../absorb/types.js'

// --- Fixtures ---

const BASE_INPUT: AbsorbInput = {
  currentBranch: 'main',
  changedFiles: [
    { path: 'src/index.ts', status: 'modified' },
    { path: 'src/utils/helpers.ts', status: 'modified' },
    { path: 'src/components/App.tsx', status: 'added' },
    { path: 'docs/api.md', status: 'added' },
    { path: 'docs/guide.md', status: 'modified' },
    { path: 'tests/unit.test.ts', status: 'added' },
    { path: 'package.json', status: 'modified' },
  ],
  existingBranches: ['main', 'dev'],
}

const EMPTY_INPUT: AbsorbInput = {
  currentBranch: 'main',
  changedFiles: [],
  existingBranches: ['main'],
}

const SINGLE_FILE_INPUT: AbsorbInput = {
  currentBranch: 'feature',
  changedFiles: [{ path: 'README.md', status: 'modified' }],
  existingBranches: ['feature'],
}

// --- Strategy: by-directory ---

describe('Strategy: by-directory', () => {
  test('groups 7 files into 3+ branches by directory', () => {
    const plan = strategyByDirectory(BASE_INPUT)

    expect(plan.strategy).toBe('by-directory')
    expect(plan.branches.length).toBeGreaterThanOrEqual(3)
    expect(plan.unassigned.length).toBe(0)

    const totalFiles = plan.branches.reduce((s, b) => s + b.files.length, 0)
    expect(totalFiles).toBe(7)
  })

  test('src/ files go to same branch', () => {
    const plan = strategyByDirectory(BASE_INPUT)

    const srcBranch = plan.branches.find((b) => b.branchName.includes('src'))
    expect(srcBranch).toBeDefined()
    expect(srcBranch?.files.length).toBe(3)
    expect(srcBranch?.files.every((f) => f.path.startsWith('src/'))).toBe(true)
  })

  test('docs/ files go to same branch', () => {
    const plan = strategyByDirectory(BASE_INPUT)

    const docsBranch = plan.branches.find((b) => b.branchName.includes('docs'))
    expect(docsBranch).toBeDefined()
    expect(docsBranch?.files.length).toBe(2)
  })

  test('branch names are prefixed with current branch', () => {
    const plan = strategyByDirectory(BASE_INPUT)

    for (const branch of plan.branches) {
      expect(branch.branchName.startsWith('main/')).toBe(true)
    }
  })

  test('each file has a reason', () => {
    const plan = strategyByDirectory(BASE_INPUT)

    for (const branch of plan.branches) {
      for (const file of branch.files) {
        expect(file.reason).toBeTruthy()
        expect(typeof file.reason).toBe('string')
      }
    }
  })

  test('summary contains strategy name', () => {
    const plan = strategyByDirectory(BASE_INPUT)
    expect(plan.summary).toContain('by-directory')
    expect(plan.summary).toContain('7 file')
  })

  test('no null fields in plan', () => {
    const plan = strategyByDirectory(BASE_INPUT)

    expect(plan.strategy).not.toBeNull()
    expect(plan.branches).not.toBeNull()
    expect(plan.unassigned).not.toBeNull()
    expect(plan.summary).not.toBeNull()

    for (const branch of plan.branches) {
      expect(branch.branchName).not.toBeNull()
      expect(branch.branchName).toBeTruthy()
      expect(branch.files).not.toBeNull()
      expect(typeof branch.confidence).toBe('number')
    }
  })
})

// --- Strategy: by-type ---

describe('Strategy: by-type', () => {
  test('groups files by type category', () => {
    const plan = strategyByType(BASE_INPUT)

    expect(plan.strategy).toBe('by-type')
    expect(plan.branches.length).toBeGreaterThanOrEqual(2)

    const totalFiles = plan.branches.reduce((s, b) => s + b.files.length, 0)
    expect(totalFiles).toBe(7)
  })

  test('TypeScript files (.ts, .tsx) go to ts branch', () => {
    const plan = strategyByType(BASE_INPUT)

    const tsBranch = plan.branches.find((b) => b.branchName.includes('ts'))
    expect(tsBranch).toBeDefined()
    // src/index.ts, src/utils/helpers.ts, src/components/App.tsx, tests/unit.test.ts
    expect(tsBranch?.files.length).toBeGreaterThanOrEqual(3)
  })

  test('docs files (.md) go to docs branch', () => {
    const plan = strategyByType(BASE_INPUT)

    const docsBranch = plan.branches.find((b) => b.branchName.includes('docs'))
    expect(docsBranch).toBeDefined()
    expect(docsBranch?.files.length).toBeGreaterThanOrEqual(2) // api.md, guide.md
  })

  test('config files (.json) go to config branch', () => {
    const plan = strategyByType(BASE_INPUT)

    const configBranch = plan.branches.find((b) =>
      b.branchName.includes('config'),
    )
    expect(configBranch).toBeDefined()
    expect(configBranch?.files.length).toBe(1) // package.json
  })

  test('no null fields in plan', () => {
    const plan = strategyByType(BASE_INPUT)

    for (const branch of plan.branches) {
      expect(branch.branchName).not.toBeNull()
      expect(branch.branchName).toBeTruthy()
      for (const file of branch.files) {
        expect(file.path).not.toBeNull()
        expect(file.status).not.toBeNull()
        expect(file.reason).not.toBeNull()
      }
    }
  })
})

// --- Strategy: auto ---

describe('Strategy: auto', () => {
  test('picks a strategy and returns valid plan', () => {
    const plan = strategyAuto(BASE_INPUT)

    expect(plan.strategy).toBe('auto')
    expect(plan.branches.length).toBeGreaterThanOrEqual(2)

    const totalFiles = plan.branches.reduce((s, b) => s + b.files.length, 0)
    expect(totalFiles).toBe(7)
  })

  test('auto is never worse than individual strategies', () => {
    const autoPlan = strategyAuto(BASE_INPUT)
    const dirPlan = strategyByDirectory(BASE_INPUT)
    const typePlan = strategyByType(BASE_INPUT)

    const autoTotal = autoPlan.branches.reduce((s, b) => s + b.files.length, 0)
    const dirTotal = dirPlan.branches.reduce((s, b) => s + b.files.length, 0)
    const typeTotal = typePlan.branches.reduce((s, b) => s + b.files.length, 0)

    // Auto should assign at least as many files as the best individual
    expect(autoTotal).toBe(Math.max(dirTotal, typeTotal))
  })
})

// --- Edge cases ---

describe('Edge cases', () => {
  test('empty workspace returns empty plan', () => {
    const plan = strategyByDirectory(EMPTY_INPUT)

    expect(plan.branches.length).toBe(0)
    expect(plan.unassigned.length).toBe(0)
    expect(plan.summary).toContain('0 file')
  })

  test('single file returns single branch', () => {
    const plan = strategyByDirectory(SINGLE_FILE_INPUT)

    expect(plan.branches.length).toBe(1)
    expect(plan.branches[0].files.length).toBe(1)
    expect(plan.branches[0].files[0].path).toBe('README.md')
  })

  test('root-level files (no directory) handled correctly', () => {
    const input: AbsorbInput = {
      currentBranch: 'main',
      changedFiles: [
        { path: 'Makefile', status: 'modified' },
        { path: 'LICENSE', status: 'added' },
      ],
      existingBranches: ['main'],
    }

    const plan = strategyByDirectory(input)

    // Root files should be in a "root-files" branch
    expect(plan.branches.length).toBe(1)
    expect(plan.branches[0].branchName).toContain('root-files')
    expect(plan.branches[0].files.length).toBe(2)
  })

  test('branch name conflicts are resolved', () => {
    const input: AbsorbInput = {
      currentBranch: 'main',
      changedFiles: [{ path: 'src/a.ts', status: 'modified' }],
      existingBranches: ['main/src'], // conflict!
    }

    const plan = strategyByDirectory(input)

    expect(plan.branches.length).toBe(1)
    // Should not be "main/src" since that exists
    expect(plan.branches[0].branchName).toBe('main/src-2')
  })

  test('runStrategy dispatches correctly', () => {
    const dirPlan = runStrategy('by-directory', BASE_INPUT)
    const typePlan = runStrategy('by-type', BASE_INPUT)
    const autoPlan = runStrategy('auto', BASE_INPUT)

    expect(dirPlan.strategy).toBe('by-directory')
    expect(typePlan.strategy).toBe('by-type')
    expect(autoPlan.strategy).toBe('auto')
  })

  test('all strategies produce same total file count', () => {
    const dirPlan = strategyByDirectory(BASE_INPUT)
    const typePlan = strategyByType(BASE_INPUT)

    const dirTotal =
      dirPlan.branches.reduce((s, b) => s + b.files.length, 0) +
      dirPlan.unassigned.length
    const typeTotal =
      typePlan.branches.reduce((s, b) => s + b.files.length, 0) +
      typePlan.unassigned.length

    expect(dirTotal).toBe(7)
    expect(typeTotal).toBe(7)
  })
})
