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
import type { LogFn, PodmanRuntime } from './podman-runtime'

const COMPOSE_PROJECT_NAME = 'browseros-openclaw'
const COMPOSE_FILE_NAME = 'docker-compose.yml'
const ENV_FILE_NAME = '.env'

export class ContainerRuntime {
  constructor(
    private podman: PodmanRuntime,
    private projectDir: string,
  ) {}

  async ensureReady(onLog?: LogFn): Promise<void> {
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
    const start = Date.now()
    while (Date.now() - start < timeoutMs) {
      if (await this.isReady(port)) return true
      await Bun.sleep(1000)
    }
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
        name.startsWith('browseros-openclaw'),
      )

      if (containers.length === 0 || allOurs) {
        await this.podman.stopMachine()
      }
    } catch {
      // Best effort — don't stop machine if we can't check
    }
  }

  async execInContainer(command: string[], onLog?: LogFn): Promise<number> {
    const containerName = `${COMPOSE_PROJECT_NAME}-openclaw-gateway-1`
    return this.podman.runCommand(['exec', containerName, ...command], {
      onOutput: onLog,
    })
  }

  private async compose(args: string[], onLog?: LogFn): Promise<number> {
    return this.podman.runCommand(['compose', ...args], {
      cwd: this.projectDir,
      env: { COMPOSE_PROJECT_NAME },
      onOutput: onLog,
    })
  }
}
