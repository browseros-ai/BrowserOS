/**
 * @license
 * Copyright 2025 BrowserOS
 */

import { afterEach, beforeEach, describe, expect, it } from 'bun:test'
import { existsSync } from 'node:fs'
import { mkdir, mkdtemp, rm, stat, writeFile } from 'node:fs/promises'
import { dirname, join, resolve } from 'node:path'
import { ContainerCli } from '../../src/lib/container'
import { LimaCli, type VmManifest, VmRuntime } from '../../src/lib/vm'
import {
  getCachedManifestPath,
  getContainerdSocketPath,
  VM_NAME,
} from '../../src/lib/vm/paths'

const LIVE_VM_SMOKE_TIMEOUT_MS = 10 * 60 * 1000
const liveIt = process.env.LIVE_VM_SMOKE === '1' ? it : it.skip
const limactlPath = process.env.LIMACTL_PATH ?? 'limactl'
const templatePath = resolve(
  import.meta.dir,
  '../../../../packages/build-tools/template/browseros-vm.yaml',
)

const manifest: VmManifest = {
  schemaVersion: 2,
  updatedAt: '2026-04-22T00:00:00.000Z',
  agents: {},
}

describe('BrowserOS VM live smoke', () => {
  let root: string
  let limaHome: string

  beforeEach(async () => {
    root = await mkdtemp('/tmp/bovm-')
    limaHome = join(root, 'lima')
    const manifestPath = getCachedManifestPath(root)
    await mkdir(dirname(manifestPath), { recursive: true })
    await writeFile(manifestPath, `${JSON.stringify(manifest, null, 2)}\n`)
  })

  afterEach(async () => {
    if (process.env.LIVE_VM_SMOKE === '1') {
      await new LimaCli({ limactlPath, limaHome })
        .delete(VM_NAME)
        .catch(() => undefined)
    }
    await rm(root, { recursive: true, force: true })
  })

  liveIt(
    'creates, starts, uses, stops, and deletes the BrowserOS Lima VM',
    async () => {
      expect(existsSync(templatePath)).toBe(true)
      const runtime = new VmRuntime({
        limactlPath,
        limaHome,
        templatePath,
        browserosRoot: root,
        readinessTimeoutMs: 5 * 60 * 1000,
        readinessPollMs: 1000,
      })
      const cli = new ContainerCli({
        limactlPath,
        limaHome,
        vmName: VM_NAME,
      })

      await runtime.ensureReady()
      expect((await stat(getContainerdSocketPath(root))).isSocket()).toBe(true)
      const nerdctlInfoOutput: string[] = []
      const nerdctlInfo = await cli.runCommand(['info'], (line) =>
        nerdctlInfoOutput.push(line),
      )
      if (nerdctlInfo.exitCode !== 0) {
        throw new Error(
          `nerdctl info failed with exit ${nerdctlInfo.exitCode}:\n${nerdctlInfoOutput.join('\n')}`,
        )
      }

      await cli.pullImage('docker.io/library/hello-world:latest')

      const secondStart = Date.now()
      await runtime.ensureReady()
      expect(Date.now() - secondStart).toBeLessThan(10_000)

      await runtime.stopVm()
      const vm = (await new LimaCli({ limactlPath, limaHome }).list()).find(
        (entry) => entry.name === VM_NAME,
      )
      expect(vm?.status).toBe('Stopped')
    },
    LIVE_VM_SMOKE_TIMEOUT_MS,
  )
})
