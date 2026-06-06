/**
 * @license AGPL-3.0-or-later
 * Copyright 2026 TRIOS
 *
 * TRIOS Absorb Engine — Orchestrates smart file sorting
 *
 * Flow:
 * 1. Gather workspace status (changed files + branches)
 * 2. Run selected strategy to produce an AbsorbPlan
 * 3. If dryRun: return plan only
 * 4. If execute: create branches, stage files, return results
 */

import type { GitButlerMcpClient } from '../clients/gitbutler-client.js'
import { runStrategy, type StrategyName } from './strategies.js'
import type { AbsorbInput, AbsorbPlan, AbsorbResult } from './types.js'

export interface AbsorbEngineDeps {
  gitbutler: GitButlerMcpClient
}

/**
 * Plan an absorb: analyze workspace and produce a sorting plan.
 * No side effects — safe for dryRun.
 */
export async function planAbsorb(
  deps: AbsorbEngineDeps,
  strategy: StrategyName,
): Promise<AbsorbPlan> {
  const [status, branches] = await Promise.all([
    deps.gitbutler.getStatus(),
    deps.gitbutler.getBranches(),
  ])

  // Collect all changed files
  const changedFiles: AbsorbInput['changedFiles'] = [
    ...status.staged.map((f) => ({ path: f.path, status: f.status })),
    ...status.unstaged.map((f) => ({ path: f.path, status: f.status })),
    ...status.untracked.map((p) => ({ path: p, status: 'untracked' as const })),
  ]

  // Deduplicate by path (prefer staged status)
  const seen = new Map<string, AbsorbInput['changedFiles'][number]>()
  for (const f of changedFiles) {
    if (!seen.has(f.path)) {
      seen.set(f.path, f)
    }
  }

  const input: AbsorbInput = {
    currentBranch: status.branch,
    changedFiles: [...seen.values()],
    existingBranches: branches.map((b) => b.name),
  }

  // Handle empty workspace
  if (input.changedFiles.length === 0) {
    return {
      strategy,
      branches: [],
      unassigned: [],
      summary: 'No changed files to sort — workspace is clean.',
    }
  }

  return runStrategy(strategy, input)
}

/**
 * Execute an absorb plan: create branches and stage files.
 * Returns results per branch.
 */
export async function executeAbsorb(
  deps: AbsorbEngineDeps,
  plan: AbsorbPlan,
): Promise<AbsorbResult> {
  if (plan.branches.length === 0) {
    return {
      ok: false,
      reason: 'No branches in plan — nothing to execute.',
    }
  }

  const branchesCreated: string[] = []
  let filesStaged = 0
  const errors: string[] = []

  for (const branch of plan.branches) {
    try {
      // Create branch
      await deps.gitbutler.createBranch(branch.branchName)
      branchesCreated.push(branch.branchName)

      // Stage files for this branch
      const filePaths = branch.files.map((f) => f.path)
      await deps.gitbutler.stage(filePaths)
      filesStaged += filePaths.length
    } catch (err) {
      errors.push(
        `${branch.branchName}: ${err instanceof Error ? err.message : String(err)}`,
      )
    }
  }

  if (errors.length > 0 && branchesCreated.length === 0) {
    return {
      ok: false,
      reason: `All branches failed:\n${errors.join('\n')}`,
    }
  }

  const warning = errors.length > 0 ? `\n⚠️ Errors:\n${errors.join('\n')}` : ''

  return {
    ok: true,
    reason: `Sorted ${filesStaged} file(s) into ${branchesCreated.length} branch(es).${warning}`,
    plan,
    branchesCreated,
    filesStaged,
  }
}

/**
 * Main entry: plan + optionally execute.
 */
export async function absorbSmart(
  deps: AbsorbEngineDeps,
  strategy: StrategyName,
  dryRun: boolean,
): Promise<AbsorbResult> {
  const plan = await planAbsorb(deps, strategy)

  if (dryRun) {
    return {
      ok: true,
      reason: `Dry run — plan generated. Set dryRun=false to execute.\n\n${plan.summary}`,
      plan,
    }
  }

  return executeAbsorb(deps, plan)
}
