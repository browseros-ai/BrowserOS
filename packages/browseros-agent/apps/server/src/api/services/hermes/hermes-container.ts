/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Singleton wrapper around `HermesContainer` (the `ManagedContainer`
 * subclass living in `lib/container/managed/`). Preserves the
 * existing `HermesContainerService` public surface — `prewarm`,
 * `start`, `stop`, `restart`, `shutdown`, `getAccessor`, `configure`
 * — so the existing callers (`main.ts`, `server.ts`,
 * `agent-harness-service`) compile unchanged in this PR.
 *
 * The wrapper is a temporary bridge: in a follow-up PR the
 * singleton vends the `HermesContainer` directly, the accessor
 * collapses, and `configure` becomes the constructor option of a
 * fresh instance instead of a mutating call. Goal here is minimal
 * blast-radius for the primitives + abstract base introduction.
 */

import { join } from 'node:path'
import {
  HERMES_CONTAINER_NAME,
  HERMES_IMAGE,
} from '@browseros/shared/constants/hermes'
import { getBrowserosDir } from '../../../lib/browseros-dir'
import {
  ContainerCli,
  HermesContainer,
  ImageLoader,
} from '../../../lib/container'
import { logger } from '../../../lib/logger'
import {
  getLimaHomeDir,
  resolveBundledLimactl,
  resolveBundledLimaTemplate,
  VM_NAME,
  VmRuntime,
} from '../../../lib/vm'
import { getHermesHarnessHostDir, getHermesHostStateDir } from './hermes-paths'

const UNSUPPORTED_PLATFORM_MESSAGE =
  'browseros-vm currently supports macOS only; see the Linux/Windows tracking issue'

export interface HermesContainerServiceConfig {
  resourcesDir?: string
  browserosDir?: string
}

/**
 * Structural type returned by `getAccessor()` so the ACP runtime
 * can build its `nerdctl exec` command without a hard dep on this
 * module. Mirrors `HermesGatewayAccessor` in `acpx-runtime.ts`;
 * `buildExecArgv` delegates to the underlying
 * `ManagedContainer.buildExecArgv` so the limactl/nerdctl argv
 * chain has exactly one owner.
 */
export interface HermesAccessor {
  getContainerName(): string
  getLimaHomeDir(): string
  getLimactlPath(): string
  getVmName(): string
  buildExecArgv(spec: {
    argv: readonly [string, ...string[]]
    env?: Record<string, string>
  }): string
}

export class HermesContainerService {
  private container: HermesContainer | null = null
  private limactlPath: string
  private limaHome: string
  private resourcesDir: string | null
  private browserosDir: string
  private readonly hermesStateDir: string
  private readonly platform: NodeJS.Platform

  constructor(config: HermesContainerServiceConfig = {}) {
    this.resourcesDir = config.resourcesDir ?? null
    this.browserosDir = config.browserosDir ?? getBrowserosDir()
    this.hermesStateDir = getHermesHostStateDir(this.browserosDir)
    this.platform = process.platform
    this.limactlPath = this.resolveLimactlPath()
    this.limaHome = getLimaHomeDir(this.browserosDir)
    this.initContainer()
  }

  configure(config: HermesContainerServiceConfig): void {
    let runtimeChanged = false
    if (
      config.resourcesDir !== undefined &&
      config.resourcesDir !== this.resourcesDir
    ) {
      this.resourcesDir = config.resourcesDir
      runtimeChanged = true
    }
    if (
      config.browserosDir !== undefined &&
      config.browserosDir !== this.browserosDir
    ) {
      this.browserosDir = config.browserosDir
      runtimeChanged = true
    }
    if (runtimeChanged) {
      this.limactlPath = this.resolveLimactlPath()
      this.limaHome = getLimaHomeDir(this.browserosDir)
      this.initContainer()
    }
  }

  /** Warm the VM and Hermes image so first-use spawns avoid pulls. */
  async prewarm(onLog?: (msg: string) => void): Promise<void> {
    if (!this.isSupportedPlatform()) {
      logger.warn('Hermes prewarm skipped: unsupported platform', {
        platform: this.platform,
      })
      return
    }
    await this.requireContainer().install({ onLog })
  }

  /** Bring the long-running idle container up. */
  async start(onLog?: (msg: string) => void): Promise<void> {
    if (!this.isSupportedPlatform()) {
      logger.warn('Hermes start skipped: unsupported platform', {
        platform: this.platform,
      })
      return
    }
    await this.requireContainer().start({ onLog })
  }

  async stop(): Promise<void> {
    if (!this.isSupportedPlatform()) return
    await this.requireContainer().stop()
  }

  async restart(onLog?: (msg: string) => void): Promise<void> {
    await this.stop()
    await this.start(onLog)
  }

  /** Best-effort container removal at server shutdown. */
  async shutdown(): Promise<void> {
    if (!this.isSupportedPlatform()) return
    try {
      await this.requireContainer().stop()
    } catch {
      // best effort
    }
  }

  /**
   * Live-getters used by AcpxRuntime to spawn `hermes acp` inside
   * the container. Kept structural so the ACP runtime doesn't need
   * to import this module. `buildExecArgv` delegates to the
   * underlying `HermesContainer` so the limactl/nerdctl argv chain
   * has exactly one owner; the four legacy getters are kept for
   * tests and any caller still constructing the chain by hand.
   */
  getAccessor(): HermesAccessor {
    return {
      getContainerName: () => HERMES_CONTAINER_NAME,
      getLimaHomeDir: () => this.limaHome,
      getLimactlPath: () => this.limactlPath,
      getVmName: () => VM_NAME,
      buildExecArgv: (spec) => {
        const container = this.requireContainer()
        return container.buildExecArgv(spec)
      },
    }
  }

  /**
   * Hand the underlying `HermesContainer` out so callers that have
   * been ported to the new abstraction can use its richer surface
   * (`buildExecArgv`, state subscriptions, path translation).
   * Returns `null` on unsupported platforms.
   */
  getContainer(): HermesContainer | null {
    return this.container
  }

  // ── Internal ─────────────────────────────────────────────────────

  private isSupportedPlatform(): boolean {
    return this.platform === 'darwin'
  }

  private resolveLimactlPath(): string {
    if (!this.isSupportedPlatform()) return 'limactl'
    return this.resourcesDir
      ? resolveBundledLimactl(this.resourcesDir)
      : 'limactl'
  }

  private initContainer(): void {
    if (!this.isSupportedPlatform()) {
      this.container = null
      return
    }
    const vm = new VmRuntime({
      limactlPath: this.limactlPath,
      limaHome: this.limaHome,
      templatePath: this.resourcesDir
        ? resolveBundledLimaTemplate(this.resourcesDir)
        : undefined,
      browserosRoot: this.browserosDir,
    })
    const cli = new ContainerCli({
      limactlPath: this.limactlPath,
      limaHome: this.limaHome,
      vmName: VM_NAME,
    })
    const loader = new ImageLoader(cli)
    this.container = new HermesContainer(
      {
        cli,
        loader,
        vm,
        limactlPath: this.limactlPath,
        limaHome: this.limaHome,
        vmName: VM_NAME,
        lockDir: join(this.hermesStateDir, '.locks'),
      },
      {
        hermesHarnessHostDir: getHermesHarnessHostDir(this.browserosDir),
      },
    )
    logger.debug('HermesContainer initialised', { image: HERMES_IMAGE })
  }

  private requireContainer(): HermesContainer {
    if (!this.container) throw unsupportedPlatformError()
    return this.container
  }
}

function unsupportedPlatformError(): Error {
  return new Error(UNSUPPORTED_PLATFORM_MESSAGE)
}

let service: HermesContainerService | null = null

export function configureHermesContainerService(
  config: HermesContainerServiceConfig,
): HermesContainerService {
  if (!service) {
    service = new HermesContainerService(config)
    return service
  }
  service.configure(config)
  return service
}

export function getHermesContainerService(): HermesContainerService {
  if (!service) service = new HermesContainerService()
  return service
}
