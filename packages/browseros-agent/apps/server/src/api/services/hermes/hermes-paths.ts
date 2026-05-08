/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Host-side path helpers for the Hermes container.
 *
 * Hermes per-agent state lives under the BrowserOS-managed VM state
 * directory (so it's reachable inside the Lima VM via the existing
 * vm/ → /mnt/browseros/vm bind mount). The Hermes container then bind-
 * mounts the guest-side path (/mnt/browseros/vm/hermes/harness) into
 * /data/agents/harness, so `HERMES_HOME` ends up pointing at a path
 * the container can actually open.
 */

import { mkdir, writeFile } from 'node:fs/promises'
import { join } from 'node:path'
import { getVmStateDir } from '../../../lib/browseros-dir'

/** Top-level Hermes state directory: `<browserosDir>/vm/hermes`. */
export function getHermesHostStateDir(browserosDir?: string): string {
  return join(
    browserosDir ? join(browserosDir, 'vm') : getVmStateDir(),
    'hermes',
  )
}

/** Per-agent harness root: `<browserosDir>/vm/hermes/harness`. */
export function getHermesHarnessHostDir(browserosDir?: string): string {
  return join(getHermesHostStateDir(browserosDir), 'harness')
}

/**
 * Per-agent home directory on the host. The Hermes container reads
 * `config.yaml` + `.env` from here via the harness bind mount; both
 * files are written at agent-create time by AgentHarnessService and
 * stay constant across turns.
 */
export function getHermesAgentHomeHostDir(input: {
  browserosDir?: string
  agentId: string
}): string {
  return join(
    getHermesHarnessHostDir(input.browserosDir),
    input.agentId,
    'home',
  )
}

/**
 * Write a Hermes per-agent provider config into the on-host home dir.
 * The dir lives under <browserosDir>/vm/hermes/harness/<agentId>/home/
 * which is bind-mounted into the container at /data/agents/harness/<id>/home/.
 *
 * Idempotent: writes always overwrite (last-write-wins). The provider
 * id, env var name, and credentials must be supplied by the caller —
 * Hermes agents always carry their own config; there is no
 * `~/.hermes/` fallback.
 */
export async function writeHermesPerAgentProvider(input: {
  browserosDir?: string
  agentId: string
  providerId: string
  envVarName: string
  apiKey: string
  modelId: string
  baseUrl?: string
}): Promise<void> {
  const home = getHermesAgentHomeHostDir({
    browserosDir: input.browserosDir,
    agentId: input.agentId,
  })
  await mkdir(home, { recursive: true })

  // Hermes' `provider: custom` requires a `base_url` — without one the
  // model loader rejects with `unknown provider 'custom'`. Callers that
  // use a named Hermes provider (e.g. anthropic, openrouter) can omit
  // baseUrl and Hermes resolves the URL itself.
  if (input.providerId === 'custom' && !input.baseUrl) {
    throw new Error(
      'Hermes provider "custom" requires base_url; set HermesProviderMapping.defaultBaseUrl or supply input.baseUrl',
    )
  }
  const yamlLines = [
    'model:',
    `  default: ${JSON.stringify(input.modelId)}`,
    `  provider: ${JSON.stringify(input.providerId)}`,
  ]
  if (input.baseUrl) {
    yamlLines.push(`  base_url: ${JSON.stringify(input.baseUrl)}`)
  }
  yamlLines.push('')
  await writeFile(join(home, 'config.yaml'), yamlLines.join('\n'), {
    mode: 0o600,
  })

  const envLines: string[] = [`${input.envVarName}=${input.apiKey}`, '']
  await writeFile(join(home, '.env'), envLines.join('\n'), { mode: 0o600 })
}
