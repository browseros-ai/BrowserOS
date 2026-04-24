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
  command?: string[]
}

export interface LogLine {
  stream: 'stdout' | 'stderr'
  line: string
}
