/**
 * @license AGPL-3.0-or-later
 * Copyright 2026 TRIOS
 *
 * TRIOS Absorb Smart — Sorting Strategies
 *
 * Three strategies for grouping changed files into branches:
 * 1. by-directory — groups by top-level directory
 * 2. by-type — groups by file extension/role
 * 3. auto — picks the strategy with best distribution
 */

import type {
  AbsorbInput,
  AbsorbPlan,
  PlannedBranch,
  PlannedFile,
} from './types.js'

// --- Helpers ---

/** Extract top-level directory from a file path */
function topDir(filePath: string): string {
  const parts = filePath.split('/')
  return parts.length > 1 ? parts[0] : '.'
}

/** Extract file extension (without dot) */
function extension(filePath: string): string {
  const lastDot = filePath.lastIndexOf('.')
  if (lastDot === -1) return 'no-ext'
  return filePath.slice(lastDot + 1).toLowerCase()
}

/** Sanitize a string into a valid git branch name component */
function toBranchName(raw: string): string {
  return raw
    .replace(/[^a-zA-Z0-9._-]/g, '-')
    .replace(/-+/g, '-')
    .replace(/^-|-$/g, '')
    .toLowerCase()
}

/** Build a PlannedFile from input */
function toPlannedFile(
  f: AbsorbInput['changedFiles'][number],
  reason: string,
): PlannedFile {
  return { path: f.path, status: f.status, reason }
}

/** Compute a confidence score based on group distribution */
function computeConfidence(branches: PlannedBranch[], total: number): number {
  if (total === 0) return 0
  const assigned = branches.reduce((sum, b) => sum + b.files.length, 0)
  const ratio = assigned / total
  // More groups with even distribution = higher confidence
  const groupCount = branches.length
  const idealSize = total / Math.max(groupCount, 1)
  const variance = branches.reduce(
    (sum, b) => sum + (b.files.length - idealSize) ** 2,
    0,
  )
  const evenness = 1 - Math.min(variance / (total * total + 1), 1)
  return Math.round(ratio * 0.6 + evenness * 0.4 + Number.EPSILON)
}

// --- Strategy: by-directory ---

/** Group files by their top-level directory */
export function strategyByDirectory(input: AbsorbInput): AbsorbPlan {
  const groups = new Map<string, PlannedFile[]>()

  for (const f of input.changedFiles) {
    const dir = topDir(f.path)
    if (!groups.has(dir)) {
      groups.set(dir, [])
    }
    groups.get(dir)?.push(toPlannedFile(f, `in ${dir}/ directory`))
  }

  const branches: PlannedBranch[] = []
  const unassigned: PlannedFile[] = []

  for (const [dir, files] of groups) {
    const branchName =
      dir === '.'
        ? `${input.currentBranch}/root-files`
        : `${input.currentBranch}/${toBranchName(dir)}`

    // Ensure unique branch name
    const uniqueName = ensureUnique(branchName, input.existingBranches)

    branches.push({
      branchName: uniqueName,
      files,
      confidence: 0,
    })
  }

  // Compute confidence
  const total = input.changedFiles.length
  for (const b of branches) {
    b.confidence = computeConfidence(branches, total)
  }

  const summary = buildSummary('by-directory', branches, unassigned)

  return { strategy: 'by-directory', branches, unassigned, summary }
}

// --- Strategy: by-type ---

/** Map of extensions to semantic categories */
const TYPE_CATEGORIES: Record<string, string> = {
  // TypeScript / JavaScript
  ts: 'ts',
  tsx: 'ts',
  js: 'ts',
  jsx: 'ts',
  mjs: 'ts',
  // Styles
  css: 'styles',
  scss: 'styles',
  sass: 'styles',
  less: 'styles',
  // Docs
  md: 'docs',
  mdx: 'docs',
  txt: 'docs',
  adoc: 'docs',
  // Config
  json: 'config',
  yaml: 'config',
  yml: 'config',
  env: 'config',
  // Tests
  test: 'tests',
  spec: 'tests',
  // Rust
  rs: 'rust',
  toml: 'config',
  // HTML
  html: 'html',
  htm: 'html',
  // Images
  png: 'assets',
  jpg: 'assets',
  jpeg: 'assets',
  svg: 'assets',
  gif: 'assets',
  ico: 'assets',
  // Shell
  sh: 'scripts',
  bash: 'scripts',
  zsh: 'scripts',
}

/** Group files by their type/category */
export function strategyByType(input: AbsorbInput): AbsorbPlan {
  const groups = new Map<string, PlannedFile[]>()

  for (const f of input.changedFiles) {
    const ext = extension(f.path)
    const category = TYPE_CATEGORIES[ext] || 'misc'
    if (!groups.has(category)) {
      groups.set(category, [])
    }
    groups.get(category)?.push(toPlannedFile(f, `${ext} file (${category})`))
  }

  const branches: PlannedBranch[] = []
  const unassigned: PlannedFile[] = []

  for (const [category, files] of groups) {
    const branchName = `${input.currentBranch}/${toBranchName(category)}`
    const uniqueName = ensureUnique(branchName, input.existingBranches)

    branches.push({
      branchName: uniqueName,
      files,
      confidence: 0,
    })
  }

  const total = input.changedFiles.length
  for (const b of branches) {
    b.confidence = computeConfidence(branches, total)
  }

  const summary = buildSummary('by-type', branches, unassigned)

  return { strategy: 'by-type', branches, unassigned, summary }
}

// --- Strategy: auto ---

/** Pick the best strategy automatically based on heuristics */
export function strategyAuto(input: AbsorbInput): AbsorbPlan {
  const dirPlan = strategyByDirectory(input)
  const typePlan = strategyByType(input)

  const dirScore = scorePlan(dirPlan)
  const typeScore = scorePlan(typePlan)

  if (dirScore >= typeScore) {
    return { ...dirPlan, strategy: 'auto' }
  }
  return { ...typePlan, strategy: 'auto' }
}

/** Score a plan: higher is better */
function scorePlan(plan: AbsorbPlan): number {
  if (plan.branches.length === 0) return 0

  // Prefer: more branches (more separation), higher confidence, fewer unassigned
  const branchScore = Math.min(plan.branches.length, 5) / 5 // cap at 5
  const avgConfidence =
    plan.branches.reduce((s, b) => s + b.confidence, 0) / plan.branches.length
  const unassignedPenalty = plan.unassigned.length * 0.1

  // Penalize single-file branches (too granular)
  const singleFileBranches = plan.branches.filter(
    (b) => b.files.length === 1,
  ).length
  const granularityPenalty = singleFileBranches * 0.05

  return (
    branchScore * 0.3 +
    avgConfidence * 0.4 -
    unassignedPenalty -
    granularityPenalty
  )
}

// --- Strategy dispatcher ---

export type StrategyName = 'by-directory' | 'by-type' | 'auto'

export function runStrategy(
  name: StrategyName,
  input: AbsorbInput,
): AbsorbPlan {
  switch (name) {
    case 'by-directory':
      return strategyByDirectory(input)
    case 'by-type':
      return strategyByType(input)
    case 'auto':
      return strategyAuto(input)
    default:
      return strategyAuto(input)
  }
}

// --- Utilities ---

function ensureUnique(name: string, existing: string[]): string {
  if (!existing.includes(name)) return name
  let i = 2
  while (existing.includes(`${name}-${i}`)) i++
  return `${name}-${i}`
}

function buildSummary(
  strategy: string,
  branches: PlannedBranch[],
  unassigned: PlannedFile[],
): string {
  const totalFiles = branches.reduce((s, b) => s + b.files.length, 0)
  const lines = [
    `Strategy: ${strategy}`,
    `${totalFiles} file(s) → ${branches.length} branch(es)`,
  ]

  for (const b of branches) {
    lines.push(
      `  📂 ${b.branchName} (${b.files.length} file${b.files.length !== 1 ? 's' : ''})`,
    )
    for (const f of b.files) {
      lines.push(`     - ${f.path} [${f.status}]`)
    }
  }

  if (unassigned.length > 0) {
    lines.push(`  ⚠️ ${unassigned.length} unassigned file(s)`)
  }

  return lines.join('\n')
}
