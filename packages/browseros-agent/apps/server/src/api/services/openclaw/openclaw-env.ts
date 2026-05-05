/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { join, relative, resolve, sep } from 'node:path'

const STATE_DIR_NAME = '.openclaw'

/**
 * Path-traversal guard for `agent.name` before it gets joined into
 * the host workspace directory. The name is user-supplied at
 * agent-create time, and `path.join` happily resolves `..` /
 * absolute segments — so a name like `../../tmp` would point the
 * workspace at the user's home directory, the harness's pre-turn
 * snapshot would walk it, and `produced_files` rows would point at
 * arbitrary host paths that subsequent download / preview routes
 * would then serve as "agent outputs".
 *
 * Reject anything that isn't a flat, single-segment name composed
 * of safe filename characters. The check is intentionally
 * conservative — agent names are short slugs in practice.
 */
export function isAgentWorkspaceNameSafe(name: string): boolean {
  if (typeof name !== 'string') return false
  const trimmed = name.trim()
  if (trimmed === '' || trimmed === '.' || trimmed === '..') return false
  // No path separators, no NULs, no control chars (charCode < 0x20).
  for (let i = 0; i < trimmed.length; i++) {
    const code = trimmed.charCodeAt(i)
    if (code < 0x20) return false
  }
  if (/[\\/]/.test(trimmed)) return false
  // No `..` segments and no leading dot (avoid hidden / dotfile escapes).
  if (trimmed.startsWith('.')) return false
  if (trimmed.includes('..')) return false
  return true
}

export function getOpenClawStateDir(openclawDir: string): string {
  return join(openclawDir, STATE_DIR_NAME)
}

export function getOpenClawStateConfigPath(openclawDir: string): string {
  return join(getOpenClawStateDir(openclawDir), 'openclaw.json')
}

export function getOpenClawStateEnvPath(openclawDir: string): string {
  return join(getOpenClawStateDir(openclawDir), '.env')
}

export function getHostWorkspaceDir(
  openclawDir: string,
  agentName: string,
): string {
  if (agentName !== 'main' && !isAgentWorkspaceNameSafe(agentName)) {
    throw new Error(
      `Refusing to compute workspace dir for unsafe agent name: ${agentName}`,
    )
  }
  const stateDir = getOpenClawStateDir(openclawDir)
  const candidate = resolve(
    stateDir,
    agentName === 'main' ? 'workspace' : `workspace-${agentName}`,
  )
  // Defensive containment check: even with a safe-looking name the
  // resolved path must live under the state dir. If it doesn't,
  // refuse rather than return a path the caller would then trust.
  const stateDirResolved = resolve(stateDir)
  const rel = relative(stateDirResolved, candidate)
  if (rel === '' || rel.startsWith('..') || rel.startsWith(`..${sep}`)) {
    throw new Error(
      `Resolved workspace dir escapes openclaw state dir: ${candidate}`,
    )
  }
  return candidate
}

export function mergeEnvContent(
  current: string,
  updates: Record<string, string>,
): { changed: boolean; content: string } {
  if (Object.keys(updates).length === 0) {
    return {
      changed: false,
      content: normalizeEnvContent(current),
    }
  }

  const lines = current === '' ? [] : current.replace(/\r\n/g, '\n').split('\n')
  const nextLines = [...lines]
  let changed = false

  for (const [key, value] of Object.entries(updates)) {
    const replacement = `${key}=${value}`
    const index = nextLines.findIndex((line) => line.startsWith(`${key}=`))
    if (index === -1) {
      nextLines.push(replacement)
      changed = true
      continue
    }
    if (nextLines[index] === replacement) {
      continue
    }
    nextLines[index] = replacement
    changed = true
  }

  const content = normalizeEnvContent(nextLines.join('\n'))
  return {
    changed: changed || content !== normalizeEnvContent(current),
    content,
  }
}

function normalizeEnvContent(content: string): string {
  const trimmed = content.trim()
  return trimmed ? `${trimmed}\n` : ''
}
