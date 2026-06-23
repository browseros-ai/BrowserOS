/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import {
  type HostCommandRunner,
  type ResolvedHostBinary,
  resolveHostBinary,
  runHostCommand,
} from './binary-resolver'
import { resolveBundledBun } from './bundled-bun'
import { resolveBundledNativeBinary } from './bundled-native-binary'
import {
  HOST_ACP_ADAPTER_CONFIG,
  type HostAcpAdapter,
  hasAcpPackageConfig,
} from './config'
import { probeNpxPackageCache } from './npx-package-cache'

export { probeNpxPackageCache } from './npx-package-cache'

export type AdapterReadiness =
  | 'ready'
  | 'needs-auth'
  | 'needs-install'
  | 'will-fetch-package'
  | 'diagnostic-warning'
  | 'unknown'

export type AdapterInstallState =
  | 'installed'
  | 'npx-available'
  | 'package-runner-available'
  | 'not-installed'

export type NativeCliState = 'present' | 'missing' | 'unknown'
export type AdapterAuthState =
  | 'authenticated'
  | 'unauthenticated'
  | 'not-applicable'
  | 'unknown'
export type AdapterLaunchSource =
  | 'bundled-bun'
  | 'host-npx'
  | 'host-cli'
  | 'runtime'
  | 'none'
export type PackageCacheState = 'cached' | 'fetch-required' | 'unknown'

export interface AdapterHealth {
  healthy: boolean
  reason?: string
  checkedAt: number
  readiness: AdapterReadiness
  installState: AdapterInstallState
  nativeCliState: NativeCliState
  authState: AdapterAuthState
  version?: string
  adapterLaunchSource: AdapterLaunchSource
  packageCacheState: PackageCacheState
}

export interface DetectHostAdapterOptions {
  env?: NodeJS.ProcessEnv
  platform?: NodeJS.Platform
  resourcesDir?: string | null
  timeoutMs?: number
  now?: () => number
  resolveBinary?: (name: string) => Promise<ResolvedHostBinary | null>
  runCommand?: HostCommandRunner
  probePackageCache?: (
    packageName: string,
    versionRange?: string,
  ) => Promise<boolean>
  resolveBundledBun?: typeof resolveBundledBun
  resolveBundledNativeBinary?: typeof resolveBundledNativeBinary
}

const DEFAULT_PROBE_TIMEOUT_MS = 3_000

/** Detects whether a host ACP adapter can be launched, versioned, and authenticated. */
export async function detectHostAdapter(
  adapter: HostAcpAdapter,
  options: DetectHostAdapterOptions = {},
): Promise<AdapterHealth> {
  const config = HOST_ACP_ADAPTER_CONFIG[adapter]
  const platform = options.platform ?? process.platform
  const env = options.env ?? process.env
  const timeoutMs = options.timeoutMs ?? DEFAULT_PROBE_TIMEOUT_MS
  const now = options.now ?? Date.now
  const runCommand = options.runCommand ?? runHostCommand
  const resolveBinary =
    options.resolveBinary ??
    ((name: string) => resolveHostBinary(name, { env, platform, timeoutMs }))
  const probePackageCache =
    options.probePackageCache ??
    ((packageName: string, versionRange?: string) =>
      probeNpxPackageCache(packageName, { versionRange }))
  const resolveBun = options.resolveBundledBun ?? resolveBundledBun
  const resolveBundledNative =
    options.resolveBundledNativeBinary ?? resolveBundledNativeBinary

  const nativeCli = await resolveNativeCli({
    adapter,
    nativeBinary: config.nativeBinary,
    resourcesDir: options.resourcesDir,
    env,
    platform,
    resolveBinary,
    resolveBundledNativeBinary: resolveBundledNative,
  }).catch(() => null)
  const launch = await detectAdapterLaunch({
    adapter,
    nativeCli,
    platform,
    resourcesDir: options.resourcesDir,
    resolveBinary,
    probePackageCache,
    resolveBundledBun: resolveBun,
  })

  const nativeCliState: NativeCliState = nativeCli ? 'present' : 'missing'
  let version: string | undefined
  let versionProbeOk = false
  let authState: AdapterAuthState = 'unknown'
  if (nativeCli) {
    const probes = await Promise.all([
      probeVersion(nativeCli, runCommand, timeoutMs),
      probeAuth(adapter, nativeCli, runCommand, timeoutMs),
    ])
    versionProbeOk = probes[0].ok
    version = probes[0].version
    authState = probes[1]
  }
  const launchKind = hasAcpPackageConfig(config) ? 'package' : 'host-cli'
  const installState = determineInstallState({
    nativeCliState,
    adapterLaunchSource: launch.source,
    packageCacheState: launch.packageCacheState,
  })
  const readiness = determineReadiness({
    authState,
    launch,
    launchKind,
    versionProbeOk,
  })
  const reason = reasonFor({
    displayName: config.displayName,
    launchKind,
    readiness,
    versionProbeOk,
  })

  return {
    healthy: readiness === 'ready' || readiness === 'diagnostic-warning',
    ...(reason ? { reason } : {}),
    checkedAt: now(),
    readiness,
    installState,
    nativeCliState,
    authState,
    ...(version ? { version } : {}),
    adapterLaunchSource: launch.source,
    packageCacheState: launch.packageCacheState,
  }
}

/** Resolves BrowserOS-packaged native CLIs before consulting the user's host PATH. */
async function resolveNativeCli(input: {
  adapter: HostAcpAdapter
  nativeBinary: string
  resourcesDir?: string | null
  env: NodeJS.ProcessEnv
  platform: NodeJS.Platform
  resolveBinary: (name: string) => Promise<ResolvedHostBinary | null>
  resolveBundledNativeBinary: typeof resolveBundledNativeBinary
}): Promise<ResolvedHostBinary | null> {
  const bundled = input.resolveBundledNativeBinary({
    adapter: input.adapter,
    resourcesDir: input.resourcesDir,
    env: input.env,
    platform: input.platform,
  })
  if (bundled) return bundled
  return input.resolveBinary(input.nativeBinary)
}

async function detectAdapterLaunch(input: {
  adapter: HostAcpAdapter
  nativeCli: ResolvedHostBinary | null
  platform: NodeJS.Platform
  resourcesDir?: string | null
  resolveBinary: (name: string) => Promise<ResolvedHostBinary | null>
  probePackageCache: (
    packageName: string,
    versionRange?: string,
  ) => Promise<boolean>
  resolveBundledBun: typeof resolveBundledBun
}): Promise<{
  source: AdapterLaunchSource
  packageCacheState: PackageCacheState
}> {
  const config = HOST_ACP_ADAPTER_CONFIG[input.adapter]
  if (!hasAcpPackageConfig(config)) {
    return {
      source: input.nativeCli ? 'host-cli' : 'none',
      packageCacheState: 'unknown',
    }
  }

  const bundledBun = input.resolveBundledBun({
    resourcesDir: input.resourcesDir,
    platform: input.platform,
  })
  if (bundledBun) {
    return { source: 'bundled-bun', packageCacheState: 'unknown' }
  }

  const npx = await input.resolveBinary('npx').catch(() => null)
  if (!npx) return { source: 'none', packageCacheState: 'unknown' }

  const cached = await input
    .probePackageCache(config.acpPackageName, config.acpPackageVersionRange)
    .catch(() => false)
  return {
    source: 'host-npx',
    packageCacheState: cached ? 'cached' : 'fetch-required',
  }
}

async function probeVersion(
  binary: ResolvedHostBinary,
  runCommand: HostCommandRunner,
  timeoutMs: number,
): Promise<{ ok: boolean; version?: string }> {
  const result = await runCommand(binary.path, ['--version'], {
    env: binary.env,
    timeoutMs,
  }).catch(() => null)
  if (result?.exitCode !== 0) return { ok: false }
  const firstLine = result.stdout.trim().split(/\r?\n/)[0]?.trim()
  return {
    ok: true,
    ...(firstLine ? { version: firstLine } : {}),
  }
}

async function probeAuth(
  adapter: HostAcpAdapter,
  binary: ResolvedHostBinary,
  runCommand: HostCommandRunner,
  timeoutMs: number,
): Promise<AdapterAuthState> {
  if (adapter === 'claude') {
    const result = await runCommand(binary.path, ['auth', 'status'], {
      env: binary.env,
      timeoutMs,
    }).catch(() => null)
    if (!result) return 'unknown'
    return result.exitCode === 0 ? 'authenticated' : 'unauthenticated'
  }

  const timeoutPerCodexAuthProbe = splitTimeoutForCodexFallback(timeoutMs)
  const status = await runCommand(binary.path, ['login', 'status'], {
    env: binary.env,
    timeoutMs: timeoutPerCodexAuthProbe,
  }).catch(() => null)
  if (status) {
    if (status.exitCode === 0) return 'authenticated'
    if (!isUnsupportedCodexStatus(status)) return 'unauthenticated'
  }

  const doctor = await runCommand(binary.path, ['doctor'], {
    env: binary.env,
    timeoutMs: timeoutPerCodexAuthProbe,
  }).catch(() => null)
  if (!doctor) return 'unknown'
  const output = `${doctor.stdout}\n${doctor.stderr}`.toLowerCase()
  if (
    output.includes('not authenticated') ||
    output.includes('not logged in') ||
    output.includes('no authentication')
  ) {
    return 'unauthenticated'
  }
  return doctor.exitCode === 0 ? 'authenticated' : 'unknown'
}

function splitTimeoutForCodexFallback(timeoutMs: number): number {
  return Math.max(1, Math.floor(timeoutMs / 2))
}

function isUnsupportedCodexStatus(result: {
  stdout: string
  stderr: string
}): boolean {
  const output = `${result.stdout}\n${result.stderr}`.toLowerCase()
  return (
    output.includes('unrecognized') ||
    output.includes('unknown command') ||
    output.includes('unexpected argument') ||
    output.includes('invalid subcommand')
  )
}

function determineInstallState(input: {
  nativeCliState: NativeCliState
  adapterLaunchSource: AdapterLaunchSource
  packageCacheState: PackageCacheState
}): AdapterInstallState {
  if (
    input.nativeCliState === 'present' ||
    input.packageCacheState === 'cached'
  ) {
    return 'installed'
  }
  if (input.adapterLaunchSource === 'host-npx') return 'npx-available'
  if (input.adapterLaunchSource === 'bundled-bun') {
    return 'package-runner-available'
  }
  return 'not-installed'
}

function determineReadiness(input: {
  authState: AdapterAuthState
  launch: { source: AdapterLaunchSource; packageCacheState: PackageCacheState }
  launchKind: 'package' | 'host-cli'
  versionProbeOk: boolean
}): AdapterReadiness {
  if (input.launch.source === 'none') return 'needs-install'
  if (input.launchKind === 'host-cli' && !input.versionProbeOk) return 'unknown'
  if (input.authState === 'unauthenticated') return 'needs-auth'
  if (input.launch.packageCacheState === 'fetch-required') {
    return 'will-fetch-package'
  }
  if (input.authState === 'unknown') return 'diagnostic-warning'
  return 'ready'
}

function reasonFor(input: {
  displayName: string
  launchKind: 'package' | 'host-cli'
  readiness: AdapterReadiness
  versionProbeOk: boolean
}): string | undefined {
  switch (input.readiness) {
    case 'needs-auth':
      return `${input.displayName} is installed but is not authenticated.`
    case 'needs-install':
      if (input.launchKind === 'host-cli') {
        return `${input.displayName} CLI is not installed.`
      }
      return `${input.displayName} adapter package cannot launch because neither bundled Bun nor npx is available.`
    case 'will-fetch-package':
      return `${input.displayName} adapter package will be downloaded on first use.`
    case 'diagnostic-warning':
      return `${input.displayName} can launch, but authentication could not be verified.`
    case 'unknown':
      if (input.launchKind === 'host-cli' && !input.versionProbeOk) {
        return `${input.displayName} CLI was found but failed its version probe.`
      }
      return `${input.displayName} readiness could not be checked.`
    default:
      return undefined
  }
}
