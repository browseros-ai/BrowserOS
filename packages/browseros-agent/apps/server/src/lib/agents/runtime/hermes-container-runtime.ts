/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Hermes-specific runtime. Owns the container spec, readiness probe,
 * mount roots, ACP launch spec, and per-turn context prep — the full
 * adapter surface lives in this single class.
 */

import { mkdir } from 'node:fs/promises'
import { join } from 'node:path'
import {
  HERMES_CONTAINER_HARNESS_DIR,
  HERMES_CONTAINER_NAME,
  HERMES_IMAGE,
} from '@browseros/shared/constants/hermes'
import {
  getHermesAgentHomeHostDir,
  getHermesHarnessHostDir,
  getHermesHostStateDir,
} from '../../../api/services/hermes/hermes-paths'
import { getBrowserosDir } from '../../browseros-dir'
import { ContainerCli } from '../../container/container-cli'
import { ImageLoader } from '../../container/image-loader'
import type {
  ContainerDescriptor,
  ManagedContainerDeps,
  MountRoot,
} from '../../container/managed'
import type { ContainerSpec } from '../../container/types'
import { logger } from '../../logger'
import {
  GUEST_VM_STATE,
  getLimaHomeDir,
  resolveBundledLimactl,
  resolveBundledLimaTemplate,
  VM_NAME,
  VmRuntime,
} from '../../vm'
import type {
  PrepareAcpxAgentContextInput,
  PreparedAcpxAgentContext,
} from '../acpx-agent-adapter'
import {
  finishBrowserosManagedContext,
  prepareBrowserosManagedContext,
} from '../acpx-agent-common'
import { ContainerAgentRuntime } from './container-agent-runtime'
import { getAgentRuntimeRegistry } from './registry'
import type { ExecSpec } from './types'

const HERMES_BINARY = '/opt/hermes/.venv/bin/hermes'

export interface HermesContainerRuntimeConfig {
  /** BrowserOS state root — used to compute per-agent home paths. */
  browserosDir: string
  /** Host-side directory where Hermes per-agent home dirs live. */
  hermesHarnessHostDir: string
}

export class HermesContainerRuntime extends ContainerAgentRuntime {
  readonly descriptor: ContainerDescriptor & { kind: 'container' } = {
    adapterId: 'hermes',
    displayName: 'Hermes',
    kind: 'container',
    defaultImage: HERMES_IMAGE,
    containerName: HERMES_CONTAINER_NAME,
    platforms: ['darwin'],
    // Hermes has no HTTP probe; we exec `hermes --version` instead
    // (see `readinessProbe` below). Generous timeout because the
    // first exec inside a freshly-started container can be slow.
    readinessProbe: { timeoutMs: 30_000, intervalMs: 500 },
  }

  private readonly hermesConfig: HermesContainerRuntimeConfig

  constructor(
    deps: ManagedContainerDeps,
    config: HermesContainerRuntimeConfig,
  ) {
    super(deps)
    this.hermesConfig = config
  }

  // ── ManagedContainer abstracts ───────────────────────────────────

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
        HERMES_BINARY,
        '--version',
      ])
      return exitCode === 0
    } catch {
      return false
    }
  }

  // ── AgentRuntime additions ───────────────────────────────────────

  getPerAgentHomeDir(agentId: string): string {
    return getHermesAgentHomeHostDir({
      browserosDir: this.hermesConfig.browserosDir,
      agentId,
    })
  }

  /**
   * ExecSpec for `hermes acp`. The dispatcher feeds this to
   * `buildExecArgv()` (inherited from `ManagedContainer`) to get the
   * launch command string. PYTHONUNBUFFERED is re-added defensively —
   * the container has it set too, but acpx spawns through `nerdctl
   * exec` which doesn't inherit container env onto the new process.
   */
  getAcpExecSpec(commandEnv: Record<string, string>): ExecSpec {
    return {
      argv: [HERMES_BINARY, 'acp'],
      env: { PYTHONUNBUFFERED: '1', ...commandEnv },
    }
  }

  /** Per-turn context prep — thin wrapper around the standalone
   *  `prepareHermesContext` so callers that prefer the runtime-style
   *  surface stay self-contained. */
  prepareTurnContext(
    input: PrepareAcpxAgentContextInput,
  ): Promise<PreparedAcpxAgentContext> {
    return prepareHermesContext(input)
  }
}

/**
 * Translate a host-side hermes home path to its in-container equivalent.
 * The container bind-mounts `<browserosDir>/vm/hermes/harness` (host)
 * onto `/data/agents/harness` (container), so paths under the host
 * harness root map cleanly to `/data/agents/harness/...` inside.
 *
 * Returns the original host path when it doesn't sit under the harness
 * root — defensive escape hatch for tests that inject a custom dir.
 */
function translateHermesHomeToContainerPath(
  hostHome: string,
  browserosDir: string,
): string {
  const harnessHostRoot = getHermesHarnessHostDir(browserosDir)
  if (hostHome === harnessHostRoot) return HERMES_CONTAINER_HARNESS_DIR
  if (hostHome.startsWith(`${harnessHostRoot}/`)) {
    return `${HERMES_CONTAINER_HARNESS_DIR}${hostHome.slice(harnessHostRoot.length)}`
  }
  return hostHome
}

/**
 * Prepares Hermes with a per-agent HERMES_HOME under
 * `<browserosDir>/vm/hermes/harness/<id>/home`. Provider config
 * (config.yaml + .env) is written into this directory at agent-create
 * time by AgentHarnessService.writeHermesPerAgentProvider. There is no
 * fallback to a global `~/.hermes/` install — Hermes agents always
 * carry their own provider config.
 *
 * HERMES_HOME inside the container is the container-side path
 * (`/data/agents/harness/<id>/home`) so Hermes resolves it correctly
 * when the runtime spawns `hermes acp` via `nerdctl exec`.
 *
 * Pure function — no runtime instance required, used directly by
 * the per-adapter prepare router in `acpx-agent-adapter.ts`.
 */
export async function prepareHermesContext(
  input: PrepareAcpxAgentContextInput,
): Promise<PreparedAcpxAgentContext> {
  const common = await prepareBrowserosManagedContext(input)

  // Hermes-specific home lives under vm/ so it's reachable inside the
  // Lima VM; the shared `common.paths.agentHome` (under agents/harness)
  // is OUTSIDE the VM mount and would not be visible to nerdctl.
  const hermesAgentHome = getHermesAgentHomeHostDir({
    browserosDir: input.browserosDir,
    agentId: input.agent.id,
  })
  await mkdir(hermesAgentHome, { recursive: true })

  const hermesAgentHomeInContainer = translateHermesHomeToContainerPath(
    hermesAgentHome,
    input.browserosDir,
  )

  return finishBrowserosManagedContext({
    ...common,
    commandEnv: {
      HERMES_HOME: hermesAgentHomeInContainer,
    },
    // Hermes runs inside a Lima container; the BrowserOS HTTP MCP
    // server lives on the host. `host.containers.internal` resolves
    // to the VM gateway (via --add-host on the hermes container) so
    // hermes can reach the MCP endpoint that the harness injects via
    // newSession.
    browserosMcpHost: 'host.containers.internal',
  })
}

// ── Factory + wire-up ──────────────────────────────────────────────

export interface ConfigureHermesRuntimeOptions {
  /** Bundled-resources root (provided by the launcher); when set,
   *  resolves bundled limactl + Lima template paths instead of host
   *  defaults. Optional in tests. */
  resourcesDir?: string
  /** Override BrowserOS state dir (defaults to `getBrowserosDir()`). */
  browserosDir?: string
}

/**
 * Build a `HermesContainerRuntime` with production deps (bundled
 * limactl, BrowserOS state dirs, Lima VM runtime) and register it in
 * the global `AgentRuntimeRegistry`. Returns `null` on non-darwin —
 * the harness checks for the runtime and falls back gracefully.
 *
 * Idempotent against accidental double-init only insofar as the
 * registry's duplicate guard fires; callers should call this once at
 * server startup.
 */
export function configureHermesRuntime(
  options: ConfigureHermesRuntimeOptions = {},
): HermesContainerRuntime | null {
  if (process.platform !== 'darwin') {
    logger.warn('Hermes runtime skipped: unsupported platform', {
      platform: process.platform,
    })
    return null
  }

  const browserosDir = options.browserosDir ?? getBrowserosDir()
  const resourcesDir = options.resourcesDir ?? null
  const limactlPath = resourcesDir
    ? resolveBundledLimactl(resourcesDir)
    : 'limactl'
  const limaHome = getLimaHomeDir(browserosDir)
  const hermesStateDir = getHermesHostStateDir(browserosDir)
  const hermesHarnessHostDir = getHermesHarnessHostDir(browserosDir)

  const vm = new VmRuntime({
    limactlPath,
    limaHome,
    templatePath: resourcesDir
      ? resolveBundledLimaTemplate(resourcesDir)
      : undefined,
    browserosRoot: browserosDir,
  })
  const cli = new ContainerCli({ limactlPath, limaHome, vmName: VM_NAME })
  const loader = new ImageLoader(cli)

  const runtime = new HermesContainerRuntime(
    {
      cli,
      loader,
      vm,
      limactlPath,
      limaHome,
      vmName: VM_NAME,
      lockDir: join(hermesStateDir, '.locks'),
    },
    { browserosDir, hermesHarnessHostDir },
  )

  getAgentRuntimeRegistry().register(runtime)
  logger.debug('HermesContainerRuntime registered', { image: HERMES_IMAGE })
  return runtime
}

/** Convenience getter — returns the registered runtime or null. */
export function getHermesRuntime(): HermesContainerRuntime | null {
  const r = getAgentRuntimeRegistry().get('hermes')
  return r instanceof HermesContainerRuntime ? r : null
}
