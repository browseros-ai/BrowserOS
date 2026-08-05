/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Constructs the spawn command for a built-in ACP adapter. Prefers the BrowserOS-shipped Bun at
 * <resourcesDir>/bin/third_party/bun so end-user installs without Node
 * still have a working launcher; falls back to the existing
 * `npx -y …` command when the bundled binary is unavailable
 * (development configurations, third_party not shipped, platforms
 * outside darwin / linux / win32).
 */

import type { AcpAgentType } from '@browseros/shared/schemas/agent'
import { resolveBundledBun, withBundledBunAcpAdapterEnv } from './bundled-bun'
import { withBundledNativeBinaryPath } from './bundled-native-binary'
import { HOST_ACP_ADAPTER_CONFIG } from './config'

export type AcpLauncherSource = 'bundled-bun' | 'host-npx-fallback'

export interface AcpLauncherResolution {
  command: string
  source: AcpLauncherSource
}

export interface ResolveAcpSpawnCommandInput {
  agentType: AcpAgentType
  browserosDir?: string | null
  env?: NodeJS.ProcessEnv
  resourcesDir?: string | null
  platform?: NodeJS.Platform
  resolveBundledBun?: typeof resolveBundledBun
}

export function resolveAcpSpawnCommand(
  input: ResolveAcpSpawnCommandInput,
): AcpLauncherResolution {
  const config = HOST_ACP_ADAPTER_CONFIG[input.agentType]

  const resolve = input.resolveBundledBun ?? resolveBundledBun
  const bunPath = resolve({
    resourcesDir: input.resourcesDir,
    platform: input.platform,
  })
  if (bunPath) {
    return {
      command: wrapCommandWithEnv(
        `${quoteAcpCommandToken(bunPath)} x --bun --silent --package ${quoteAcpCommandToken(config.acpPackageSpec)} ${quoteAcpCommandToken(config.acpBin)}`,
        withBundledNativeBinaryPath({
          resourcesDir: input.resourcesDir,
          env: withBundledBunAcpAdapterEnv({
            bunPath,
            browserosDir: input.browserosDir,
            env: input.env,
            platform: input.platform,
          }),
          platform: input.platform,
        }),
      ),
      source: 'bundled-bun',
    }
  }
  const hostPath = inheritedPath(input.env ?? process.env, input.platform)
  const hostEnv = withBundledNativeBinaryPath({
    resourcesDir: input.resourcesDir,
    env: hostPath,
    platform: input.platform,
  })
  if (
    pathValue(hostEnv, input.platform) !== pathValue(hostPath, input.platform)
  ) {
    return {
      command: wrapCommandWithEnv(config.acpCommand, hostEnv),
      source: 'host-npx-fallback',
    }
  }
  return { command: config.acpCommand, source: 'host-npx-fallback' }
}

function inheritedPath(
  env: NodeJS.ProcessEnv,
  platform: NodeJS.Platform = process.platform,
): Record<string, string> {
  const key =
    platform === 'win32'
      ? (Object.keys(env).find((name) => name.toLowerCase() === 'path') ??
        'Path')
      : 'PATH'
  return env[key] ? { [key]: env[key] } : {}
}

function pathValue(
  env: Record<string, string>,
  platform: NodeJS.Platform = process.platform,
): string | undefined {
  const key =
    platform === 'win32'
      ? Object.keys(env).find((name) => name.toLowerCase() === 'path')
      : 'PATH'
  return key ? env[key] : undefined
}

/** Quotes a token for acpx command splitting while preserving Windows backslashes. */
function quoteAcpCommandToken(value: string): string {
  return `'${value.replace(/'/g, "'\\''")}'`
}

function wrapCommandWithEnv(
  command: string,
  env: Record<string, string>,
): string {
  const prefix = Object.entries(env)
    .sort(([left], [right]) => left.localeCompare(right))
    .map(([key, value]) => `${key}=${quoteAcpCommandToken(value)}`)
    .join(' ')
  return prefix ? `env ${prefix} ${command}` : command
}
