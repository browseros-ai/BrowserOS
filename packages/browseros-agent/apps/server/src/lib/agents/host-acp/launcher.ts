/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { AcpAgentType } from '@browseros/shared/schemas/agent'
import { resolveBundledBun, withBundledBunAcpAdapterEnv } from './bundled-bun'
import { withBundledNativeBinaryPath } from './bundled-native-binary'
import { HOST_ACP_ADAPTER_CONFIG } from './config'

export type AcpLauncherSource = 'bundled-bun' | 'host-npx-fallback'

export interface AcpLauncherResolution {
  argv: string[]
  source: AcpLauncherSource
}

export interface ResolveAcpSpawnCommandInput {
  agentType: AcpAgentType
  browserosDir?: string | null
  env?: NodeJS.ProcessEnv
  resourcesDir?: string | null
  platform?: NodeJS.Platform
  spawnEnv?: Readonly<Record<string, string>>
  resolveBundledBun?: typeof resolveBundledBun
}

export function resolveAcpSpawnCommand(
  input: ResolveAcpSpawnCommandInput,
): AcpLauncherResolution {
  const config = HOST_ACP_ADAPTER_CONFIG[input.agentType]
  const platform = input.platform ?? process.platform

  const resolve = input.resolveBundledBun ?? resolveBundledBun
  const bunPath = resolve({
    resourcesDir: input.resourcesDir,
    platform,
  })
  if (bunPath) {
    return {
      argv: withSpawnEnvironment(
        [
          bunPath,
          'x',
          '--bun',
          '--silent',
          '--package',
          config.acpPackageSpec,
          config.acpBin,
        ],
        {
          ...withBundledNativeBinaryPath({
            resourcesDir: input.resourcesDir,
            env: withBundledBunAcpAdapterEnv({
              bunPath,
              browserosDir: input.browserosDir,
              env: input.env,
              platform,
            }),
            platform,
          }),
          ...input.spawnEnv,
        },
        platform,
        bunPath,
      ),
      source: 'bundled-bun',
    }
  }
  const hostPath = inheritedPath(input.env ?? process.env, platform)
  const hostEnv = withBundledNativeBinaryPath({
    resourcesDir: input.resourcesDir,
    env: hostPath,
    platform,
  })
  const bundledNativePathAdded =
    pathValue(hostEnv, platform) !== pathValue(hostPath, platform)
  const spawnEnv = {
    ...(bundledNativePathAdded ? hostEnv : {}),
    ...input.spawnEnv,
  }

  return {
    argv: withSpawnEnvironment([...config.acpArgv], spawnEnv, platform, 'node'),
    source: 'host-npx-fallback',
  }
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

const ENVIRONMENT_LAUNCHER_SOURCE = [
  "const { spawn } = require('node:child_process')",
  "const payload = JSON.parse(Buffer.from(process.argv[1], 'base64url').toString('utf8'))",
  "const child = spawn(payload.argv[0], payload.argv.slice(1), { env: { ...process.env, ...payload.env }, stdio: 'inherit', windowsHide: true })",
  "child.once('error', error => { console.error(error); process.exit(1) })",
  "child.once('exit', code => process.exit(code ?? 1))",
  "for (const signal of ['SIGINT', 'SIGTERM']) process.on(signal, () => child.kill(signal))",
].join(';')

function withSpawnEnvironment(
  argv: string[],
  env: Record<string, string>,
  platform: NodeJS.Platform,
  environmentRunner: string,
): string[] {
  const entries = Object.entries(env).sort(([left], [right]) =>
    left.localeCompare(right),
  )
  if (entries.length === 0) return argv
  if (platform !== 'win32') {
    return ['env', ...entries.map(([key, value]) => `${key}=${value}`), ...argv]
  }
  const payload = Buffer.from(JSON.stringify({ argv, env })).toString(
    'base64url',
  )
  return [environmentRunner, '--eval', ENVIRONMENT_LAUNCHER_SOURCE, payload]
}
