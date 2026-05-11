/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

/**
 * acpx configuration sync: read, import, and probe custom ACP agents
 * defined in an acpx config directory.
 *
 * This module has zero runtime dependencies on the acpx CLI — it reads
 * config.json directly and spawns the user's binary for probing.
 */

import { spawn } from 'node:child_process'
import { homedir } from 'node:os'
import { resolve } from 'node:path'
import type { AgentDefinition } from '../../../lib/agents/agent-types'

// ── Types ─────────────────────────────────────────────────────────

export interface AcpxConfig {
  agents: Record<string, { command: string; args?: string[] }>
}

export interface ImportResult {
  name: string
  imported: boolean
  warning?: string
}

export interface ProbeResult {
  healthy: boolean
  error?: string
}

// ── Config reader ─────────────────────────────────────────────────

const ACPX_CONFIG_DIR_ENV = 'ACPX_CONFIG_DIR'
const DEFAULT_ACPX_DIR = '~/.acpx'

/**
 * Read acpx config from the given directory (or the default). Returns
 * null when the config file is missing or unparseable.
 */
export function readAcpxConfig(acpxDir?: string): AcpxConfig | null {
  const dir = resolveAcpxDir(acpxDir)
  try {
    // Synchronous read via Bun's eager require-style won't work for
    // arbitrary paths, so we do a sync read for the route handler.
    // Using Bun.file().text() would be async; the route handlers are
    // async already, but the function signature is sync for the
    // import-acpx handler. We'll use a try/catch with require('fs').
    // eslint-disable-next-line @typescript-eslint/no-require-imports
    const fs = require('node:fs')
    const configPath = resolve(dir, 'config.json')
    if (!fs.existsSync(configPath)) return null
    const raw = fs.readFileSync(configPath, 'utf-8')
    const parsed = JSON.parse(raw)
    if (!parsed || typeof parsed !== 'object' || !parsed.agents) return null
    // Validate shape: agents must be a record of { command, args? }
    const agents: AcpxConfig['agents'] = {}
    for (const [name, entry] of Object.entries(
      parsed.agents as Record<string, unknown>,
    )) {
      if (!entry || typeof entry !== 'object') continue
      const rec = entry as Record<string, unknown>
      if (typeof rec.command !== 'string') continue
      agents[name] = {
        command: rec.command,
        args: Array.isArray(rec.args)
          ? rec.args.filter((a: unknown) => typeof a === 'string')
          : [],
      }
    }
    return { agents }
  } catch {
    return null
  }
}

function resolveAcpxDir(acpxDir?: string): string {
  const raw = acpxDir || process.env[ACPX_CONFIG_DIR_ENV] || DEFAULT_ACPX_DIR
  const expanded = raw.startsWith('~') ? resolve(homedir(), raw.slice(1)) : raw
  return expanded
}

// ── Import ────────────────────────────────────────────────────────

const BUILTIN_ADAPTER_NAMES = new Set(['claude', 'codex', 'openclaw', 'hermes'])

/**
 * Import agents from an acpx config. Returns a result per agent entry.
 * Skips agents whose names already exist in the current set (idempotent).
 * Warns when an imported agent shares a name with a built-in adapter.
 */
export function importAgentsFromAcpx(
  config: AcpxConfig,
  existingAgents: AgentDefinition[],
): ImportResult[] {
  const existingNames = new Set(existingAgents.map((a) => a.name))
  const results: ImportResult[] = []

  for (const [name, _entry] of Object.entries(config.agents)) {
    if (existingNames.has(name)) {
      results.push({ name, imported: false })
      continue
    }

    const warning = BUILTIN_ADAPTER_NAMES.has(name)
      ? 'Name conflicts with built-in agent'
      : undefined

    results.push({
      name,
      imported: true,
      warning,
    })
  }

  return results
}

// ── Probe ─────────────────────────────────────────────────────────

/** Metacharacters that indicate shell injection attempts. */
const SHELL_META_RE = /[;&|`$(){}!#~]/

const PROBE_TIMEOUT_MS = 15_000

/**
 * Probe a custom ACP binary by sending an `initialize` JSON-RPC request
 * over stdin and reading stdout for a valid response. Never throws —
 * returns `{ healthy: false, error }` on all failures.
 *
 * SECURITY: spawns with `shell: false`. Splits command into argv.
 * Rejects commands containing shell metacharacters.
 */
export function probeCustomAgent(
  command: string,
  args: string[] = [],
): Promise<ProbeResult> {
  return new Promise((resolve) => {
    // Security: reject shell metacharacters
    if (SHELL_META_RE.test(command)) {
      resolve({
        healthy: false,
        error: `Command contains unsafe characters. Use a plain binary path.`,
      })
      return
    }

    // Split command into argv (handle simple space-separated tokens)
    const argv = splitCommandToArgv(command)

    try {
      const child = spawn(argv[0], [...argv.slice(1), ...args], {
        stdio: ['pipe', 'pipe', 'pipe'],
        shell: false,
        timeout: PROBE_TIMEOUT_MS,
      })

      let stdout = ''
      let _stderr = ''
      let settled = false

      const finish = (result: ProbeResult) => {
        if (settled) return
        settled = true
        try {
          child.kill('SIGKILL')
        } catch {}
        resolve(result)
      }

      const timer = setTimeout(() => {
        const hint = command.includes('npx')
          ? ' First-run npx downloads can take time; retry in a moment.'
          : ''
        finish({
          healthy: false,
          error: `Probe timed out. Binary did not respond to ACP initialize.${hint}`,
        })
      }, PROBE_TIMEOUT_MS)

      child.stdout?.on('data', (chunk: Buffer) => {
        stdout += chunk.toString()
        // Check if we got a JSON-RPC response
        if (stdout.includes('"jsonrpc"')) {
          clearTimeout(timer)
          try {
            // Try to find a valid JSON-RPC line
            const lines = stdout.split('\n').filter(Boolean)
            for (const line of lines) {
              try {
                const parsed = JSON.parse(line)
                if (parsed.jsonrpc === '2.0') {
                  finish({ healthy: true })
                  return
                }
              } catch {
                // try next line
              }
            }
            // Got jsonrpc string but couldn't parse a valid message
          } catch {
            // ignore
          }
        }
      })

      child.stderr?.on('data', (chunk: Buffer) => {
        _stderr += chunk.toString()
      })

      child.on('error', (err) => {
        clearTimeout(timer)
        const message =
          (err as NodeJS.ErrnoException).code === 'ENOENT'
            ? `Command not found: ${command}`
            : `Failed to spawn: ${err.message}`
        finish({
          healthy: false,
          error: message,
        })
      })

      child.on('close', (code) => {
        clearTimeout(timer)
        if (settled) return
        if (code !== 0 && !stdout.includes('"jsonrpc"')) {
          finish({
            healthy: false,
            error: `Process exited with code ${code}. Is this an ACP-compliant binary?`,
          })
        } else if (!stdout.includes('"jsonrpc"')) {
          finish({
            healthy: false,
            error: `Binary responded with non-ACP output. Expected JSON-RPC.`,
          })
        }
      })

      // Send ACP initialize request
      const initRequest = JSON.stringify({
        jsonrpc: '2.0',
        id: 1,
        method: 'initialize',
        params: {
          protocolVersion: '0.1',
          capabilities: {},
          clientInfo: { name: 'browseros-probe', version: '1.0.0' },
        },
      })

      child.stdin?.write(`${initRequest}\n`)
      child.stdin?.end()
    } catch (err) {
      resolve({
        healthy: false,
        error: `Failed to spawn: ${err instanceof Error ? err.message : String(err)}`,
      })
    }
  })
}

/**
 * Split a command string into argv, handling simple space-separated
 * tokens. Does NOT handle quoting — users should provide the binary
 * path directly.
 */
function splitCommandToArgv(command: string): string[] {
  return command.trim().split(/\s+/)
}
