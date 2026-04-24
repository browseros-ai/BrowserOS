/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { cpSync, existsSync, mkdirSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { getBrowserosDir } from '../../../lib/browseros-dir'
import { ContainerCli, ImageLoader } from '../../../lib/container'
import { logger } from '../../../lib/logger'
import {
  detectArch,
  getLimaHomeDir,
  resolveBundledLimactl,
  resolveBundledLimaTemplate,
  VM_NAME,
  VmRuntime,
} from '../../../lib/vm'
import { readCachedManifest } from '../../../lib/vm/manifest'
import { VM_TELEMETRY_EVENTS } from '../../../lib/vm/telemetry'
import { ContainerRuntime } from './container-runtime'

const UNSUPPORTED_PLATFORM_MESSAGE =
  'browseros-vm currently supports macOS only; see the Linux/Windows tracking issue'

export interface ContainerRuntimeFactoryInput {
  resourcesDir?: string
  projectDir: string
  browserosRoot?: string
  platform?: NodeJS.Platform
}

export function buildContainerRuntime(
  input: ContainerRuntimeFactoryInput,
): ContainerRuntime {
  const platform = input.platform ?? process.platform
  if (platform !== 'darwin') {
    if (process.env.NODE_ENV === 'test') {
      return new UnsupportedPlatformTestRuntime(input.projectDir)
    }
    throw unsupportedPlatformError()
  }

  const browserosRoot = input.browserosRoot ?? getBrowserosDir()
  if (input.resourcesDir) {
    migrateLegacyOpenClawDirSync(browserosRoot)
  }

  const limactlPath = input.resourcesDir
    ? resolveBundledLimactl(input.resourcesDir)
    : 'limactl'
  const limaHome = getLimaHomeDir(browserosRoot)
  const vm = new VmRuntime({
    limactlPath,
    limaHome,
    templatePath: input.resourcesDir
      ? resolveBundledLimaTemplate(input.resourcesDir)
      : undefined,
    browserosRoot,
  })
  const shell = new ContainerCli({ limactlPath, limaHome, vmName: VM_NAME })
  const loader = new DeferredImageLoader(shell, browserosRoot)

  return new ContainerRuntime({
    vm,
    shell,
    loader,
    projectDir: input.projectDir,
  })
}

export async function migrateLegacyOpenClawDir(
  browserosRoot = getBrowserosDir(),
): Promise<void> {
  migrateLegacyOpenClawDirSync(browserosRoot)
}

function migrateLegacyOpenClawDirSync(browserosRoot = getBrowserosDir()): void {
  const legacyDir = join(browserosRoot, 'openclaw')
  const nextDir = join(browserosRoot, 'vm', 'openclaw')
  if (!existsSync(legacyDir)) return
  if (existsSync(nextDir)) {
    logger.warn('OpenClaw legacy and VM state directories both exist', {
      legacyDir,
      nextDir,
    })
    return
  }

  mkdirSync(dirname(nextDir), { recursive: true })
  cpSync(legacyDir, nextDir, { recursive: true })
  logger.info(VM_TELEMETRY_EVENTS.migrationOpenClawMoved, {
    from: legacyDir,
    to: nextDir,
  })
}

class DeferredImageLoader {
  constructor(
    private readonly shell: ContainerCli,
    private readonly browserosRoot: string,
  ) {}

  async ensureImageLoaded(ref: string, onLog?: (msg: string) => void) {
    const manifest = await readCachedManifest(this.browserosRoot)
    const loader = new ImageLoader(
      this.shell,
      manifest,
      detectArch(),
      this.browserosRoot,
    )
    await loader.ensureImageLoaded(ref, onLog)
  }
}

class UnsupportedPlatformTestRuntime extends ContainerRuntime {
  constructor(projectDir: string) {
    super({
      vm: {} as VmRuntime,
      shell: {} as ContainerCli,
      loader: { ensureImageLoaded: rejectUnsupportedPlatform },
      projectDir,
    })
  }

  override async ensureReady(): Promise<void> {
    throw unsupportedPlatformError()
  }

  override async isPodmanAvailable(): Promise<boolean> {
    return false
  }

  override async getMachineStatus(): Promise<{
    initialized: boolean
    running: boolean
  }> {
    return { initialized: false, running: false }
  }

  override async pullImage(): Promise<void> {
    throw unsupportedPlatformError()
  }

  override async startGateway(): Promise<void> {
    throw unsupportedPlatformError()
  }

  override async stopGateway(): Promise<void> {}

  override async restartGateway(): Promise<void> {
    throw unsupportedPlatformError()
  }

  override async getGatewayLogs(): Promise<string[]> {
    return []
  }

  override async isHealthy(): Promise<boolean> {
    return false
  }

  override async isReady(): Promise<boolean> {
    return false
  }

  override async waitForReady(): Promise<boolean> {
    return false
  }

  override async stopVm(): Promise<void> {}

  override async execInContainer(): Promise<number> {
    throw unsupportedPlatformError()
  }

  override async runGatewaySetupCommand(): Promise<number> {
    throw unsupportedPlatformError()
  }

  override tailGatewayLogs(): () => void {
    return () => {}
  }
}

async function rejectUnsupportedPlatform(): Promise<never> {
  throw unsupportedPlatformError()
}

function unsupportedPlatformError(): Error {
  return new Error(UNSUPPORTED_PLATFORM_MESSAGE)
}
