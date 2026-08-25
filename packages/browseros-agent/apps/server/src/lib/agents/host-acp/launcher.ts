/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { AcpAgentType } from '@browseros/shared/schemas/agent'
import { resolveBundledBun, withBundledBunAcpAdapterEnv } from './bundled-bun'
import { withBundledNativeBinaryPath } from './bundled-native-binary'
import { HOST_ACP_ADAPTER_CONFIG, type HostAcpAdapter } from './config'
import { splitCommandLine } from './parse-command'
import { mergePath, resolveLoginShellPath } from './resolve-login-path'

export type AcpLauncherSource = 'bundled-bun' | 'host-npx-fallback' | 'custom'

export interface AcpLauncherResolution {
  argv: string[]
  source: AcpLauncherSource
}

export interface ResolveAcpSpawnCommandInput {
  agentType: AcpAgentType
  /**
   * Full command line for `type: 'custom'` agents. Shell-split into argv and run
   * as given (no bundled-bun package wrapping).
   */
  customCommand?: string
  browserosDir?: string | null
  env?: NodeJS.ProcessEnv
  resourcesDir?: string | null
  platform?: NodeJS.Platform
  spawnEnv?: Readonly<Record<string, string>>
  resolveBundledBun?: typeof resolveBundledBun
  resolveLoginShellPath?: typeof resolveLoginShellPath
}

export function resolveAcpSpawnCommand(
  input: ResolveAcpSpawnCommandInput,
): AcpLauncherResolution {
  const platform = input.platform ?? process.platform

  if (input.agentType === 'custom' || input.customCommand !== undefined) {
    return resolveCustomSpawnCommand(input, platform)
  }

  const config = HOST_ACP_ADAPTER_CONFIG[input.agentType as HostAcpAdapter]

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
  const hostArgv =
    platform === 'win32'
      ? windowsNpxArgv([...config.acpArgv], input.env ?? process.env)
      : [...config.acpArgv]

  return {
    argv: withSpawnEnvironment(
      hostArgv,
      spawnEnv,
      platform,
      'node',
      platform === 'win32',
    ),
    source: 'host-npx-fallback',
  }
}

function resolveCustomSpawnCommand(
  input: ResolveAcpSpawnCommandInput,
  platform: NodeJS.Platform,
): AcpLauncherResolution {
  const command = input.customCommand?.trim()
  if (!command) {
    throw new Error('Custom ACP agent is missing a launch command')
  }
  const argv = splitCommandLine(command)
  if (argv.length === 0) {
    throw new Error('Custom ACP agent command is empty')
  }
  // Run the user's command as given; the child inherits process.env, and the
  // custom env rides in as spawnEnv overrides.
  const baseArgv =
    platform === 'win32' ? windowsNpxArgv(argv, input.env ?? process.env) : argv
  const spawnEnv = { ...input.spawnEnv }
  // On macOS/Linux, prepend the user's login-shell PATH so a GUI-launched
  // server finds binaries installed via the shell profile (homebrew, nvm, ...).
  const loginPath = (input.resolveLoginShellPath ?? resolveLoginShellPath)({
    platform,
    env: input.env,
  })
  if (loginPath) {
    const inherited =
      pathValue(inheritedPath(input.env ?? process.env, platform), platform) ??
      ''
    spawnEnv.PATH = mergePath(loginPath, inherited)
  }
  return {
    argv: withSpawnEnvironment(
      baseArgv,
      spawnEnv,
      platform,
      'node',
      platform === 'win32',
    ),
    source: 'custom',
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
  "const child = spawn(payload.argv[0], payload.argv.slice(1), { env: { ...process.env, ...payload.env }, stdio: 'inherit', windowsHide: true, windowsVerbatimArguments: payload.windowsVerbatimArguments === true })",
  "child.once('error', error => { console.error(error); process.exit(1) })",
  "child.once('exit', code => process.exit(code ?? 1))",
  "for (const signal of ['SIGINT', 'SIGTERM']) process.on(signal, () => child.kill(signal))",
].join(';')

function withSpawnEnvironment(
  argv: string[],
  env: Record<string, string>,
  platform: NodeJS.Platform,
  environmentRunner: string,
  windowsVerbatimArguments = false,
): string[] {
  const entries = Object.entries(env).sort(([left], [right]) =>
    left.localeCompare(right),
  )
  if (entries.length === 0 && !windowsVerbatimArguments) return argv
  if (platform !== 'win32') {
    return ['env', ...entries.map(([key, value]) => `${key}=${value}`), ...argv]
  }
  const payload = Buffer.from(
    JSON.stringify({ argv, env, windowsVerbatimArguments }),
  ).toString('base64url')
  return [environmentRunner, '--eval', ENVIRONMENT_LAUNCHER_SOURCE, payload]
}

const WINDOWS_CMD_META_CHARACTERS = /([()\][%!^"`<>&|;, *?])/gu
const WINDOWS_CMD_BACKSLASH_QUOTE = /(?=(\\+?)?)\1"/gu
const WINDOWS_CMD_TRAILING_BACKSLASH = /(?=(\\+?)?)\1$/gu

function windowsNpxArgv(argv: string[], env: NodeJS.ProcessEnv): string[] {
  const [command, ...args] = argv
  if (!command) return argv
  const shellCommand = [
    command.replace(WINDOWS_CMD_META_CHARACTERS, '^$1'),
    ...args.map((argument) =>
      `"${argument
        .replace(WINDOWS_CMD_BACKSLASH_QUOTE, '$1$1\\"')
        .replace(WINDOWS_CMD_TRAILING_BACKSLASH, '$1$1')}"`.replace(
        WINDOWS_CMD_META_CHARACTERS,
        '^$1',
      ),
    ),
  ].join(' ')
  const comSpecKey = Object.keys(env).find(
    (key) => key.toLowerCase() === 'comspec',
  )
  return [
    (comSpecKey ? env[comSpecKey] : undefined) ?? 'cmd.exe',
    '/d',
    '/s',
    '/c',
    `"${shellCommand}"`,
  ]
}
