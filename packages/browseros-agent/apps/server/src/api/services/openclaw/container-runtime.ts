/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Compose-level abstraction over PodmanRuntime.
 * Manages a single compose project for the OpenClaw gateway container.
 */

import { copyFile, writeFile } from 'node:fs/promises'
import { join } from 'node:path'
import {
  OPENCLAW_COMPOSE_PROJECT_NAME,
  OPENCLAW_GATEWAY_CONTAINER_NAME,
} from '@browseros/shared/constants/openclaw'
import { logger } from '../../../lib/logger'
import type { LogFn, PodmanRuntime } from './podman-runtime'

const COMPOSE_FILE_NAME = 'docker-compose.yml'
const ENV_FILE_NAME = '.env'

export class ContainerRuntime {
  constructor(
    private podman: PodmanRuntime,
    private projectDir: string,
  ) {}

  async ensureReady(onLog?: LogFn): Promise<void> {
    logger.info('Ensuring Podman runtime readiness')
    return this.podman.ensureReady(onLog)
  }

  async isPodmanAvailable(): Promise<boolean> {
    return this.podman.isPodmanAvailable()
  }

  async getMachineStatus(): Promise<{
    initialized: boolean
    running: boolean
  }> {
    return this.podman.getMachineStatus()
  }

  async composeUp(onLog?: LogFn): Promise<void> {
    const code = await this.compose(['up', '-d'], onLog)
    if (code !== 0) throw new Error(`compose up failed with code ${code}`)
  }

  async composeDown(onLog?: LogFn): Promise<void> {
    const code = await this.compose(['down'], onLog)
    if (code !== 0) throw new Error(`compose down failed with code ${code}`)
  }

  async composeStop(onLog?: LogFn): Promise<void> {
    const code = await this.compose(['stop'], onLog)
    if (code !== 0) throw new Error(`compose stop failed with code ${code}`)
  }

  async composeRestart(onLog?: LogFn): Promise<void> {
    const code = await this.compose(['restart'], onLog)
    if (code !== 0) throw new Error(`compose restart failed with code ${code}`)
  }

  async composePull(onLog?: LogFn): Promise<void> {
    const code = await this.compose(['pull', '--quiet'], onLog)
    if (code !== 0) throw new Error(`compose pull failed with code ${code}`)
  }

  async composeLogs(tail = 50): Promise<string[]> {
    const lines: string[] = []
    await this.compose(['logs', '--no-color', '--tail', String(tail)], (line) =>
      lines.push(line),
    )
    return lines
  }

  async isHealthy(port: number): Promise<boolean> {
    try {
      const res = await fetch(`http://127.0.0.1:${port}/healthz`)
      return res.ok
    } catch {
      return false
    }
  }

  async isReady(port: number): Promise<boolean> {
    try {
      const res = await fetch(`http://127.0.0.1:${port}/readyz`)
      return res.ok
    } catch {
      return false
    }
  }

  async waitForReady(port: number, timeoutMs = 30_000): Promise<boolean> {
    logger.info('Waiting for OpenClaw gateway readiness', { port, timeoutMs })
    const start = Date.now()
    while (Date.now() - start < timeoutMs) {
      if (await this.isReady(port)) {
        logger.info('OpenClaw gateway became ready', {
          port,
          waitMs: Date.now() - start,
        })
        return true
      }
      await Bun.sleep(1000)
    }
    logger.error('Timed out waiting for OpenClaw gateway readiness', {
      port,
      timeoutMs,
    })
    return false
  }

  async copyComposeFile(sourceTemplatePath: string): Promise<void> {
    await copyFile(sourceTemplatePath, join(this.projectDir, COMPOSE_FILE_NAME))
  }

  async writeEnvFile(content: string): Promise<void> {
    await writeFile(join(this.projectDir, ENV_FILE_NAME), content, {
      mode: 0o600,
    })
  }

  /**
   * Stops the Podman machine only if no non-BrowserOS containers are running.
   * Prevents killing the user's own Podman workloads.
   */
  async stopMachineIfSafe(): Promise<void> {
    const status = await this.podman.getMachineStatus()
    if (!status.running) return

    try {
      const containers = await this.podman.listRunningContainers()
      const allOurs = containers.every((name) =>
        name.startsWith(OPENCLAW_COMPOSE_PROJECT_NAME),
      )

      if (containers.length === 0 || allOurs) {
        await this.podman.stopMachine()
      }
    } catch {
      // Best effort — don't stop machine if we can't check
    }
  }

  async execInContainer(command: string[], onLog?: LogFn): Promise<number> {
    return this.podman.runCommand(
      ['exec', OPENCLAW_GATEWAY_CONTAINER_NAME, ...command],
      {
        onOutput: onLog,
      },
    )
  }

  private async compose(args: string[], onLog?: LogFn): Promise<number> {
    const lines: string[] = []
    const command = ['podman', 'compose', ...args].join(' ')
    logger.info('Running OpenClaw compose command', {
      command,
    })
    const code = await this.podman.runCommand(['compose', ...args], {
      cwd: this.projectDir,
      env: { COMPOSE_PROJECT_NAME: OPENCLAW_COMPOSE_PROJECT_NAME },
      onOutput: (line) => {
        lines.push(line)
        onLog?.(line)
      },
    })

    if (code !== 0) {
      logger.error('OpenClaw compose command failed', {
        command,
        exitCode: code,
        output: lines,
      })
    } else {
      logger.info('OpenClaw compose command succeeded', {
        command,
      })
    }

    return code
  }
}
