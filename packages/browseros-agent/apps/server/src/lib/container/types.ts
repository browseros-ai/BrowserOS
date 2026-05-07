/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

export type LogFn = (msg: string) => void

export interface PortMapping {
  hostIp?: string
  hostPort: number
  containerPort: number
}

export interface MountSpec {
  source: string
  target: string
  readonly?: boolean
}

export interface HealthConfig {
  cmd: string
  interval?: string
  timeout?: string
  retries?: number
}

export interface ContainerSpec {
  name: string
  image: string
  restart?: 'no' | 'unless-stopped' | 'always'
  ports?: PortMapping[]
  env?: Record<string, string>
  envFile?: string
  mounts?: MountSpec[]
  addHosts?: string[]
  health?: HealthConfig
  /**
   * Override the image's ENTRYPOINT. When set, nerdctl is invoked with
   * `--entrypoint <value>`; the `command` array is appended as args to
   * this entrypoint. Useful for keeping a service-style image alive in
   * the background (e.g. `tini -- sh -c "exec sleep infinity"`) so that
   * other code paths can `nerdctl exec` into it per turn.
   */
  entrypoint?: string
  command?: string[]
}

export interface ContainerInfo {
  id: string | null
  name: string
  image: string | null
  status: string | null
  running: boolean | null
}

export interface WaitForContainerNameReleaseOptions {
  timeoutMs?: number
  intervalMs?: number
}

export interface WaitForContainerRunningOptions {
  timeoutMs?: number
  intervalMs?: number
}

export interface LogLine {
  stream: 'stdout' | 'stderr'
  line: string
}
