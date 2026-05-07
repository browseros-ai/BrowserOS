/**
 * @license
 * Copyright 2025 BrowserOS
 */

import { afterEach, describe, expect, it } from 'bun:test'
import { mkdtempSync } from 'node:fs'
import { rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import {
  HERMES_CONTAINER_HARNESS_DIR,
  HERMES_CONTAINER_NAME,
  HERMES_IMAGE,
} from '../../../../../../packages/shared/src/constants/hermes'
import {
  HermesContainer,
  type ManagedContainerDeps,
} from '../../../../src/lib/container/managed'
import type {
  ContainerInfo,
  ContainerSpec,
} from '../../../../src/lib/container/types'

interface FakeCli {
  inspectContainer: (name: string) => Promise<ContainerInfo | null>
  removeContainer: (name: string, opts?: { force?: boolean }) => Promise<void>
  waitForContainerNameRelease: () => Promise<void>
  createContainer: (spec: ContainerSpec) => Promise<void>
  startContainer: (name: string) => Promise<void>
  waitForContainerRunning: (name: string) => Promise<void>
  exec: (name: string, cmd: string[]) => Promise<number>
}

function makeDeps(opts: {
  lockDir: string
  exec?: (name: string, cmd: string[]) => Promise<number>
}): {
  deps: ManagedContainerDeps
  getCapturedSpec: () => ContainerSpec | null
} {
  let capturedSpec: ContainerSpec | null = null
  const fakeCli = {
    inspectContainer: async (): Promise<ContainerInfo | null> => ({
      id: 'cid',
      name: HERMES_CONTAINER_NAME,
      image: HERMES_IMAGE,
      status: 'running',
      running: true,
    }),
    removeContainer: async () => {},
    waitForContainerNameRelease: async () => {},
    createContainer: async (spec: ContainerSpec) => {
      capturedSpec = spec
    },
    startContainer: async () => {},
    waitForContainerRunning: async () => {},
    exec: opts.exec ?? (async () => 0),
  } satisfies FakeCli
  const fakeLoader = {
    ensureImageLoaded: async () => {},
  }
  const fakeVm = {
    ensureReady: async () => {},
    getDefaultGateway: async () => '192.168.5.2',
  }
  const deps: ManagedContainerDeps = {
    cli: fakeCli as unknown as ManagedContainerDeps['cli'],
    loader: fakeLoader as unknown as ManagedContainerDeps['loader'],
    vm: fakeVm as unknown as ManagedContainerDeps['vm'],
    limactlPath: '/opt/homebrew/bin/limactl',
    limaHome: '/Users/dev/.browseros/lima',
    vmName: 'browseros-vm',
    lockDir: opts.lockDir,
  }
  return { deps, getCapturedSpec: () => capturedSpec }
}

describe('HermesContainer', () => {
  const tempDirs: string[] = []

  afterEach(async () => {
    await Promise.all(
      tempDirs.map((dir) => rm(dir, { recursive: true, force: true })),
    )
    tempDirs.length = 0
  })

  function mkTempDir(): string {
    const dir = mkdtempSync(join(tmpdir(), 'hermes-container-test-'))
    tempDirs.push(dir)
    return dir
  }

  it('declares the canonical Hermes descriptor', () => {
    const lockDir = mkTempDir()
    const { deps } = makeDeps({ lockDir })
    const c = new HermesContainer(deps, {
      hermesHarnessHostDir: '/host/hermes/harness',
    })

    expect(c.descriptor.adapterId).toBe('hermes')
    expect(c.descriptor.containerName).toBe(HERMES_CONTAINER_NAME)
    expect(c.descriptor.defaultImage).toBe(HERMES_IMAGE)
    expect(c.descriptor.platforms).toContain('darwin')
  })

  it('start() reaches running and runs hermes --version probe', async () => {
    const lockDir = mkTempDir()
    let probeCmd: string[] | null = null
    const { deps } = makeDeps({
      lockDir,
      exec: async (_name, cmd) => {
        probeCmd = cmd
        return 0
      },
    })
    const c = new HermesContainer(deps, {
      hermesHarnessHostDir: '/host/hermes/harness',
    })

    await c.start()

    expect(c.getState()).toBe('running')
    expect(probeCmd).toEqual(['/opt/hermes/.venv/bin/hermes', '--version'])
  })

  it('start() lands errored when probe exits non-zero', async () => {
    const lockDir = mkTempDir()
    const { deps } = makeDeps({
      lockDir,
      exec: async () => 1,
    })
    const c = new HermesContainer(deps, {
      hermesHarnessHostDir: '/host/hermes/harness',
    })

    await expect(c.start()).rejects.toThrow(/probe failed/i)
    expect(c.getState()).toBe('errored')
  })

  it('builds a ContainerSpec with idle entrypoint + harness mount + add-host', async () => {
    const lockDir = mkTempDir()
    const { deps, getCapturedSpec } = makeDeps({ lockDir })
    const c = new HermesContainer(deps, {
      hermesHarnessHostDir: '/host/hermes/harness',
    })

    await c.start()

    const spec = getCapturedSpec()
    if (!spec) throw new Error('createContainer was never called')
    expect(spec.entrypoint).toBe('/bin/sh')
    expect(spec.command).toEqual(['-c', 'exec sleep infinity'])
    expect(spec.addHosts).toContain('host.containers.internal:192.168.5.2')
    const harnessMount = spec.mounts?.find(
      (m) => m.target === HERMES_CONTAINER_HARNESS_DIR,
    )
    if (!harnessMount) throw new Error('harness mount missing')
    expect(harnessMount.source).toBe('/mnt/browseros/vm/hermes/harness')
  })

  it('toContainerPath maps host harness dir to /data/agents/harness', () => {
    const lockDir = mkTempDir()
    const { deps } = makeDeps({ lockDir })
    const c = new HermesContainer(deps, {
      hermesHarnessHostDir: '/host/hermes/harness',
    })

    expect(c.toContainerPath('/host/hermes/harness/agent-01/home/x.txt')).toBe(
      `${HERMES_CONTAINER_HARNESS_DIR}/agent-01/home/x.txt`,
    )
  })

  it('buildExecArgv produces the canonical Hermes ACP spawn string', () => {
    const lockDir = mkTempDir()
    const { deps } = makeDeps({ lockDir })
    const c = new HermesContainer(deps, {
      hermesHarnessHostDir: '/host/hermes/harness',
    })

    const out = c.buildExecArgv({
      argv: ['/opt/hermes/.venv/bin/hermes', 'acp'],
      env: { HERMES_HOME: '/data/agents/harness/a/home' },
    })
    expect(out).toContain('LIMA_HOME=/Users/dev/.browseros/lima')
    expect(out).toContain('shell --workdir / browseros-vm --')
    expect(out).toContain('nerdctl exec -i')
    expect(out).toContain(HERMES_CONTAINER_NAME)
    expect(out).toContain('/opt/hermes/.venv/bin/hermes acp')
    expect(out).toContain('-e HERMES_HOME=/data/agents/harness/a/home')
  })
})
