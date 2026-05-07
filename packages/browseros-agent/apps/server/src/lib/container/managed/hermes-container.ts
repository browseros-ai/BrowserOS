/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Hermes-specific subclass of `ManagedContainer`. Provides image,
 * container spec, readiness probe, and mount roots — all the
 * adapter-specific bits. The base class handles state, lifecycle,
 * and the gated execute family.
 *
 * Lives under `lib/container/managed/` (not `api/services/hermes/`)
 * so it has no dependency on the harness layer. The wrapper at
 * `api/services/hermes/hermes-container.ts` glues this into the
 * existing service singleton + accessor surface so callers like
 * `main.ts` / `server.ts` don't need to change in this PR.
 */

import {
  HERMES_CONTAINER_HARNESS_DIR,
  HERMES_CONTAINER_NAME,
  HERMES_IMAGE,
} from '@browseros/shared/constants/hermes'
import { GUEST_VM_STATE } from '../../vm'
import type { ContainerSpec } from '../types'
import { ManagedContainer } from './managed-container'
import type { ContainerDescriptor, MountRoot } from './types'

export interface HermesContainerConfig {
  /** Host-side directory where Hermes per-agent home dirs live. */
  hermesHarnessHostDir: string
}

export class HermesContainer extends ManagedContainer {
  readonly descriptor: ContainerDescriptor = {
    adapterId: 'hermes',
    displayName: 'Hermes',
    defaultImage: HERMES_IMAGE,
    containerName: HERMES_CONTAINER_NAME,
    platforms: ['darwin'],
    // Hermes has no HTTP probe; we exec `hermes --version` instead
    // (see `readinessProbe` below). Generous timeout because the
    // first exec inside a freshly-started container can be slow.
    readinessProbe: { timeoutMs: 30_000, intervalMs: 500 },
  }

  private readonly hermesConfig: HermesContainerConfig

  constructor(
    deps: ConstructorParameters<typeof ManagedContainer>[0],
    config: HermesContainerConfig,
  ) {
    super(deps)
    this.hermesConfig = config
  }

  protected mountRoots(): readonly MountRoot[] {
    return [
      {
        hostPath: this.hermesConfig.hermesHarnessHostDir,
        containerPath: HERMES_CONTAINER_HARNESS_DIR,
        kind: 'shared',
      },
    ]
  }

  protected async buildContainerSpec(): Promise<ContainerSpec> {
    // The bind-mount source is an in-VM path, not the host path —
    // Lima's bundled mount already exposes <browserosDir>/vm/ to the
    // VM at GUEST_VM_STATE, so nerdctl sees the harness dir at
    // `${GUEST_VM_STATE}/hermes/harness`. mountRoots() above declares
    // the *logical* host↔container mapping for path-translation use.
    const guestHarnessDir = `${GUEST_VM_STATE}/hermes/harness`
    const gateway = await this.deps.vm.getDefaultGateway()
    return {
      name: HERMES_CONTAINER_NAME,
      image: HERMES_IMAGE,
      restart: 'unless-stopped',
      env: { PYTHONUNBUFFERED: '1' },
      // host.containers.internal → VM gateway so hermes inside the
      // container can reach the BrowserOS HTTP server running on the
      // host (BrowserOS MCP /mcp).
      addHosts: [`host.containers.internal:${gateway}`],
      mounts: [
        { source: guestHarnessDir, target: HERMES_CONTAINER_HARNESS_DIR },
      ],
      // Override the upstream image's `hermes acp` ENTRYPOINT — we
      // want a long-lived idle container that we `nerdctl exec` into
      // per turn. Bypass tini (0.19.0 getopt-parses `-x` even after
      // the PROGRAM, so `tini /bin/sh -c "…"` errors).
      entrypoint: '/bin/sh',
      command: ['-c', 'exec sleep infinity'],
    }
  }

  /**
   * Container-running is already checked by the base via
   * `cli.waitForContainerRunning` before this runs. Here we add an
   * exec-based liveness check: `hermes --version` exits 0. Catches
   * the failure mode where the container daemon thinks it's running
   * but the embedded Python venv is broken or the binary is missing.
   *
   * This must NOT go through `execProcess` — that would deadlock on
   * the state gate (we're in `starting`, not `running`). Use the
   * lower-level `cli.exec` directly.
   */
  protected async readinessProbe(): Promise<boolean> {
    try {
      const exitCode = await this.deps.cli.exec(this.descriptor.containerName, [
        '/opt/hermes/.venv/bin/hermes',
        '--version',
      ])
      return exitCode === 0
    } catch {
      return false
    }
  }
}
