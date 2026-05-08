/**
 * @license
 * Copyright 2025 BrowserOS
 */

import { afterEach, beforeEach, describe, expect, it } from 'bun:test'
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
  configureHermesRuntime,
  getAgentRuntimeRegistry,
  getHermesRuntime,
  HermesContainerRuntime,
  resetAgentRuntimeRegistry,
} from '../../../../src/lib/agents/runtime'
import type {
  ManagedContainerDeps,
  MountRoot,
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

describe('HermesContainerRuntime', () => {
  const tempDirs: string[] = []

  afterEach(async () => {
    await Promise.all(
      tempDirs.map((dir) => rm(dir, { recursive: true, force: true })),
    )
    tempDirs.length = 0
    resetAgentRuntimeRegistry()
  })

  function mkTempDir(): string {
    const dir = mkdtempSync(join(tmpdir(), 'hermes-runtime-test-'))
    tempDirs.push(dir)
    return dir
  }

  function makeRuntime(extraConfig?: { browserosDir?: string }) {
    const lockDir = mkTempDir()
    const browserosDir = extraConfig?.browserosDir ?? '/host/browseros'
    const { deps, getCapturedSpec } = makeDeps({ lockDir })
    const runtime = new HermesContainerRuntime(deps, {
      browserosDir,
      hermesHarnessHostDir: `${browserosDir}/vm/hermes/harness`,
    })
    return { runtime, getCapturedSpec, browserosDir }
  }

  it('declares the canonical Hermes runtime descriptor', () => {
    const { runtime } = makeRuntime()
    expect(runtime.descriptor.adapterId).toBe('hermes')
    expect(runtime.descriptor.kind).toBe('container')
    expect(runtime.descriptor.containerName).toBe(HERMES_CONTAINER_NAME)
    expect(runtime.descriptor.defaultImage).toBe(HERMES_IMAGE)
    expect(runtime.descriptor.platforms).toContain('darwin')
  })

  it('mountRoots maps the host harness dir to /data/agents/harness', () => {
    const { runtime } = makeRuntime()
    const mounts: readonly MountRoot[] = (
      runtime as unknown as { mountRoots(): readonly MountRoot[] }
    ).mountRoots()
    expect(mounts).toEqual([
      {
        hostPath: '/host/browseros/vm/hermes/harness',
        containerPath: HERMES_CONTAINER_HARNESS_DIR,
        kind: 'shared',
      },
    ])
  })

  it('start() runs the hermes --version probe and reaches running', async () => {
    let probeCmd: string[] | null = null
    const lockDir = mkTempDir()
    const { deps } = makeDeps({
      lockDir,
      exec: async (_name, cmd) => {
        probeCmd = cmd
        return 0
      },
    })
    const runtime = new HermesContainerRuntime(deps, {
      browserosDir: '/host/browseros',
      hermesHarnessHostDir: '/host/browseros/vm/hermes/harness',
    })
    await runtime.start()
    expect(runtime.getState()).toBe('running')
    expect(probeCmd).toEqual(['/opt/hermes/.venv/bin/hermes', '--version'])
  })

  it('start() lands errored when the probe exits non-zero', async () => {
    const lockDir = mkTempDir()
    const { deps } = makeDeps({ lockDir, exec: async () => 1 })
    const runtime = new HermesContainerRuntime(deps, {
      browserosDir: '/host/browseros',
      hermesHarnessHostDir: '/host/browseros/vm/hermes/harness',
    })
    await expect(runtime.start()).rejects.toThrow(/probe failed/i)
    expect(runtime.getState()).toBe('errored')
  })

  it('builds a ContainerSpec with idle entrypoint + harness mount + add-host', async () => {
    const { runtime, getCapturedSpec } = makeRuntime()
    await runtime.start()
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

  it('getPerAgentHomeDir returns the canonical host-side home path', () => {
    const { runtime } = makeRuntime()
    expect(runtime.getPerAgentHomeDir('agent-7')).toBe(
      '/host/browseros/vm/hermes/harness/agent-7/home',
    )
  })

  it('getAcpExecSpec returns argv [hermes, acp] with PYTHONUNBUFFERED merged', () => {
    const { runtime } = makeRuntime()
    const spec = runtime.getAcpExecSpec({ HERMES_HOME: '/data/agents/x' })
    expect(spec.argv).toEqual(['/opt/hermes/.venv/bin/hermes', 'acp'])
    expect(spec.env).toEqual({
      PYTHONUNBUFFERED: '1',
      HERMES_HOME: '/data/agents/x',
    })
  })

  it('buildExecArgv produces the canonical limactl/nerdctl spawn string', () => {
    const { runtime } = makeRuntime()
    const out = runtime.buildExecArgv(
      runtime.getAcpExecSpec({ HERMES_HOME: '/data/agents/harness/a/home' }),
    )
    expect(out).toContain('LIMA_HOME=/Users/dev/.browseros/lima')
    expect(out).toContain('shell --workdir / browseros-vm --')
    expect(out).toContain('nerdctl exec -i')
    expect(out).toContain(HERMES_CONTAINER_NAME)
    expect(out).toContain('/opt/hermes/.venv/bin/hermes acp')
    expect(out).toContain('-e HERMES_HOME=/data/agents/harness/a/home')
    expect(out).toContain('-e PYTHONUNBUFFERED=1')
  })

  it('toContainerPath maps host harness paths to /data/agents/harness', () => {
    const { runtime } = makeRuntime()
    expect(
      runtime.toContainerPath(
        '/host/browseros/vm/hermes/harness/agent-01/home/x.txt',
      ),
    ).toBe(`${HERMES_CONTAINER_HARNESS_DIR}/agent-01/home/x.txt`)
  })

  describe('configureHermesRuntime', () => {
    let originalPlatform: string
    beforeEach(() => {
      originalPlatform = process.platform
    })
    afterEach(() => {
      Object.defineProperty(process, 'platform', { value: originalPlatform })
    })

    it('returns null on non-darwin and skips registration', () => {
      Object.defineProperty(process, 'platform', { value: 'linux' })
      expect(configureHermesRuntime()).toBeNull()
      expect(getHermesRuntime()).toBeNull()
    })

    it('registers the runtime in the global registry on darwin', () => {
      Object.defineProperty(process, 'platform', { value: 'darwin' })
      const browserosDir = mkTempDir()
      const runtime = configureHermesRuntime({ browserosDir })
      expect(runtime).toBeInstanceOf(HermesContainerRuntime)
      expect(getHermesRuntime()).toBe(runtime)
      expect(getAgentRuntimeRegistry().get('hermes')).toBe(runtime)
    })

    it('throws on duplicate registration via the registry guard', () => {
      Object.defineProperty(process, 'platform', { value: 'darwin' })
      const browserosDir = mkTempDir()
      configureHermesRuntime({ browserosDir })
      expect(() => configureHermesRuntime({ browserosDir })).toThrow(
        /already registered/,
      )
    })
  })
})
