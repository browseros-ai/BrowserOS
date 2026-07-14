/**
 * The I/O boundary.
 *
 * readState snapshots the workspace manifest and every requested agent
 * config file into a single State value the pure planners consume.
 * applyPlan runs a Plan's ops in dependency order with atomic writes.
 *
 * The two functions are the ONLY places outside the planner that touch
 * the filesystem for domain state. Every other module operates on
 * values.
 */

import * as fsp from 'node:fs/promises'
import * as path from 'node:path'
import {
  atomicWriteFile,
  readFileWithExistence,
} from '../_internal/atomic-write'
import { readManifest } from '../_internal/manifest'
import { pathExists } from '../_internal/paths'
import { resolveAgentMcpConfigPath } from '../agents'
import type { AgentFileState, FsOp, Plan, State } from '../planner/types'
import type { AgentId, AgentScope } from '../types'

export interface ReadStateOptions {
  scope?: AgentScope
  projectRoot?: string
  /**
   * Per-agent config path override. Bypasses the OS+catalog path
   * resolution for the given agent + scope pair. Same shape v0.0.3
   * takes.
   */
  overrides?: Partial<Record<AgentId, string>>
}

const MANIFEST_FILENAME = 'manifest.json'

/**
 * Snapshot the workspace manifest plus one config file per requested
 * (agent, scope) pair.
 *
 * `agents` filters which agent config files are included. Callers who
 * skip this argument get a State with no agent files, which is enough
 * for planRescan and planAdd but not for planLink / planUnlink /
 * planDisconnect / planRemove. Those planners throw when the agent
 * they need is not in state.agents. The API layer handles this by
 * always including the target agents in its readState call.
 */
export async function readState(
  workspaceDir: string,
  agents?: ReadonlyArray<AgentId>,
  opts: ReadStateOptions = {},
): Promise<State> {
  const manifest = await readManifest(workspaceDir)
  const scope = opts.scope ?? 'system'
  const agentFiles: AgentFileState[] = []
  for (const agent of agents ?? []) {
    const configPath =
      opts.overrides?.[agent] ??
      (await resolveAgentMcpConfigPath(agent, scope, opts.projectRoot))
    const { content, exists } = await readFileWithExistence(configPath)
    // When the file exists, the parent must too. Only stat the parent
    // in the miss case so the common path stays one stat.
    const parentExists = exists
      ? true
      : await pathExists(path.dirname(configPath))
    agentFiles.push({
      agent,
      scope,
      configPath,
      rawContent: content,
      exists,
      parentExists,
    })
  }
  return {
    workspaceDir,
    manifestPath: path.join(workspaceDir, MANIFEST_FILENAME),
    manifest,
    agents: agentFiles,
  }
}

export interface ApplyPlanResult {
  writtenPaths: string[]
  removedPaths: string[]
}

/**
 * Apply a Plan. Writes happen atomically via `<file>.tmp + rename`.
 * File removals happen after writes so a partial-failure re-run picks
 * up where it left off cleanly.
 *
 * The order matters when multiple ops target overlapping files: the
 * caller is responsible for producing a plan whose ops do not conflict.
 * All planner-produced plans already satisfy that invariant (agent
 * config first, manifest last, no duplicate paths).
 */
export async function applyPlan(plan: Plan): Promise<ApplyPlanResult> {
  const writtenPaths: string[] = []
  const removedPaths: string[] = []
  // Writes first, then removes. Manifest write typically lands last
  // because the planner produces ops in that order; we do not re-sort
  // to avoid confusing consumers who inspect a plan and depend on the
  // observed order.
  for (const op of plan.ops) {
    if (op.kind === 'writeFile') {
      await writeOp(op)
      writtenPaths.push(op.path)
    }
  }
  for (const op of plan.ops) {
    if (op.kind === 'removeFile') {
      await removeOp(op)
      removedPaths.push(op.path)
    }
  }
  return { writtenPaths, removedPaths }
}

async function writeOp(
  op: Extract<FsOp, { kind: 'writeFile' }>,
): Promise<void> {
  if (op.ensureDir) await fsp.mkdir(path.dirname(op.path), { recursive: true })
  await atomicWriteFile(op.path, op.content)
}

async function removeOp(
  op: Extract<FsOp, { kind: 'removeFile' }>,
): Promise<void> {
  try {
    await fsp.unlink(op.path)
  } catch (err) {
    if ((err as NodeJS.ErrnoException).code !== 'ENOENT') throw err
  }
}
