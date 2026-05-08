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
  ContainerNotReadyError,
  type ContainerState,
  ManagedContainer,
  type ManagedContainerDeps,
  type MountRoot,
  PathOutsideMountsError,
  ResetNotSupportedError,
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

interface FakeLoader {
  ensureImageLoaded: (ref: string) => Promise<void>
}

interface FakeVm {
  ensureReady: () => Promise<void>
  getDefaultGateway: () => Promise<string>
}

class TestContainer extends ManagedContainer {
  readonly descriptor = {
    adapterId: 'test',
    displayName: 'Test',
    defaultImage: 'docker.io/test:latest',
    containerName: 'test-container',
    platforms: ['darwin' as NodeJS.Platform],
  }

  probeOutcome: boolean | Error = true
  probeCalls = 0

  protected mountRoots(): readonly MountRoot[] {
    return [
      {
        hostPath: '/host/root',
        containerPath: '/data/root',
        kind: 'shared',
      },
    ]
  }

  protected async buildContainerSpec(): Promise<ContainerSpec> {
    return {
      name: this.descriptor.containerName,
      image: this.descriptor.defaultImage,
      env: { FOO: 'bar' },
    }
  }

  protected async readinessProbe(): Promise<boolean> {
    this.probeCalls += 1
    if (this.probeOutcome instanceof Error) throw this.probeOutcome
    return this.probeOutcome
  }

  // Expose the protected helper for one specific test.
  triggerErrored(message: string) {
    // biome-ignore lint/complexity/useLiteralKeys: protected method access for tests
    this['setState']('errored', message)
  }
}

function makeFakeDeps(opts: { lockDir: string }): ManagedContainerDeps & {
  fakeCli: FakeCli
  fakeLoader: FakeLoader
  fakeVm: FakeVm
} {
  const fakeCli: FakeCli = {
    inspectContainer: async () => ({
      id: 'cid',
      name: 'test-container',
      image: 'docker.io/test:latest',
      status: 'running',
      running: true,
    }),
    removeContainer: async () => {},
    waitForContainerNameRelease: async () => {},
    createContainer: async () => {},
    startContainer: async () => {},
    waitForContainerRunning: async () => {},
    exec: async () => 0,
  }
  const fakeLoader: FakeLoader = {
    ensureImageLoaded: async () => {},
  }
  const fakeVm: FakeVm = {
    ensureReady: async () => {},
    getDefaultGateway: async () => '192.168.5.2',
  }
  return {
    cli: fakeCli as unknown as ManagedContainerDeps['cli'],
    loader: fakeLoader as unknown as ManagedContainerDeps['loader'],
    vm: fakeVm as unknown as ManagedContainerDeps['vm'],
    limactlPath: '/opt/homebrew/bin/limactl',
    limaHome: '/Users/dev/.browseros/lima',
    vmName: 'browseros-vm',
    lockDir: opts.lockDir,
    fakeCli,
    fakeLoader,
    fakeVm,
  }
}

describe('ManagedContainer', () => {
  const tempDirs: string[] = []

  afterEach(async () => {
    await Promise.all(
      tempDirs.map((dir) => rm(dir, { recursive: true, force: true })),
    )
    tempDirs.length = 0
  })

  function mkTempDir(): string {
    const dir = mkdtempSync(join(tmpdir(), 'managed-container-test-'))
    tempDirs.push(dir)
    return dir
  }

  describe('state machine', () => {
    it('transitions through start() to running', async () => {
      const lockDir = mkTempDir()
      const deps = makeFakeDeps({ lockDir })
      const c = new TestContainer(deps)
      const transitions: ContainerState[] = []
      c.subscribeState((s) => transitions.push(s))

      expect(c.getState()).toBe('not_installed')
      await c.start()

      expect(c.getState()).toBe('running')
      // installing → starting → running (the base goes through these
      // phases on every start).
      expect(transitions).toEqual(['installing', 'starting', 'running'])
    })

    it('lands in errored when readiness probe returns false', async () => {
      const lockDir = mkTempDir()
      const deps = makeFakeDeps({ lockDir })
      const c = new TestContainer(deps)
      c.probeOutcome = false

      await expect(c.start()).rejects.toThrow(/probe failed/i)
      expect(c.getState()).toBe('errored')
      expect(c.getStatusSnapshot().lastError).toMatch(/probe failed/i)
    })

    it('stop() force-transitions to stopped even from errored', async () => {
      const lockDir = mkTempDir()
      const deps = makeFakeDeps({ lockDir })
      const c = new TestContainer(deps)
      c.probeOutcome = false
      await expect(c.start()).rejects.toThrow()
      expect(c.getState()).toBe('errored')

      await c.stop()
      expect(c.getState()).toBe('stopped')
    })

    it('install() calls vm.ensureReady before loader.ensureImageLoaded (cold-boot regression)', async () => {
      const lockDir = mkTempDir()
      const deps = makeFakeDeps({ lockDir })
      const calls: string[] = []
      deps.fakeVm.ensureReady = async () => {
        calls.push('vm.ensureReady')
      }
      deps.fakeLoader.ensureImageLoaded = async () => {
        calls.push('loader.ensureImageLoaded')
      }
      const c = new TestContainer(deps)

      await c.install()

      expect(calls).toEqual(['vm.ensureReady', 'loader.ensureImageLoaded'])
      expect(c.getState()).toBe('installed')
    })
  })

  describe('execProcess gating', () => {
    it('rejects with ContainerNotReadyError when not_installed', async () => {
      const lockDir = mkTempDir()
      const deps = makeFakeDeps({ lockDir })
      const c = new TestContainer(deps)

      await expect(
        c.execProcess({ argv: ['/bin/echo', 'hi'] }),
      ).rejects.toBeInstanceOf(ContainerNotReadyError)

      try {
        await c.execProcess({ argv: ['/bin/echo', 'hi'] })
      } catch (err) {
        if (err instanceof ContainerNotReadyError) {
          expect(err.reason).toBe('not_installed')
          expect(err.state).toBe('not_installed')
          expect(err.containerId).toBe('test-container')
        }
      }
    })

    it('rejects with reason=errored when in errored state', async () => {
      const lockDir = mkTempDir()
      const deps = makeFakeDeps({ lockDir })
      const c = new TestContainer(deps)
      c.triggerErrored('probe boom')

      try {
        await c.execProcess({ argv: ['/bin/echo', 'hi'] })
        throw new Error('unreachable')
      } catch (err) {
        expect(err).toBeInstanceOf(ContainerNotReadyError)
        if (err instanceof ContainerNotReadyError) {
          expect(err.reason).toBe('errored')
          expect(err.lastError).toBe('probe boom')
        }
      }
    })

    it('waits through starting and resolves when running', async () => {
      const lockDir = mkTempDir()
      const deps = makeFakeDeps({ lockDir })
      const c = new TestContainer(deps)
      // Skip directly to a starting state without running the start
      // pipeline, then flip to running asynchronously.
      // biome-ignore lint/complexity/useLiteralKeys: test reaches into protected
      c['setState']('starting')
      // Ensure execProcess waits, not resolves immediately.
      const execPromise = c.execProcess(
        {
          argv: ['/bin/echo', 'hi'],
          env: { FOO: 'bar' },
        },
        { execGateTimeoutMs: 1_000 },
      )
      // Flip to running on next tick — execProcess should resolve.
      setTimeout(() => {
        // biome-ignore lint/complexity/useLiteralKeys: test reaches into protected
        c['setState']('running')
      }, 10)
      const proc = await execPromise
      proc.kill()
      // Bun spawned a real process — it will exit quickly. Drain so
      // the test doesn't leak resources.
      await proc.exited.catch(() => undefined)
    })

    it('rejects with reason=timeout when starting never resolves', async () => {
      const lockDir = mkTempDir()
      const deps = makeFakeDeps({ lockDir })
      const c = new TestContainer(deps)
      // biome-ignore lint/complexity/useLiteralKeys: test reaches into protected
      c['setState']('starting')

      try {
        await c.execProcess(
          { argv: ['/bin/echo', 'hi'] },
          { execGateTimeoutMs: 50 },
        )
        throw new Error('unreachable')
      } catch (err) {
        expect(err).toBeInstanceOf(ContainerNotReadyError)
        if (err instanceof ContainerNotReadyError) {
          expect(err.reason).toBe('timeout')
        }
      }
    })
  })

  describe('buildExecArgv', () => {
    it('produces the canonical limactl/nerdctl chain', () => {
      const lockDir = mkTempDir()
      const deps = makeFakeDeps({ lockDir })
      const c = new TestContainer(deps)

      const out = c.buildExecArgv({
        argv: ['/opt/hermes/.venv/bin/hermes', 'acp'],
        env: { HERMES_HOME: '/data/agents/harness/a/home' },
      })

      // Single source of truth for the chain — pin the exact string
      // so future edits are explicit.
      expect(out).toBe(
        [
          'env',
          'LIMA_HOME=/Users/dev/.browseros/lima',
          '/opt/homebrew/bin/limactl',
          'shell',
          '--workdir',
          '/',
          'browseros-vm',
          '--',
          'nerdctl',
          'exec',
          '-i',
          '-e',
          'HERMES_HOME=/data/agents/harness/a/home',
          'test-container',
          '/opt/hermes/.venv/bin/hermes',
          'acp',
        ].join(' '),
      )
    })

    it('omits -e flags when env is empty', () => {
      const lockDir = mkTempDir()
      const deps = makeFakeDeps({ lockDir })
      const c = new TestContainer(deps)

      const out = c.buildExecArgv({ argv: ['/bin/version'] })
      expect(out).not.toContain('-e ')
      expect(out).toContain('test-container /bin/version')
    })
  })

  describe('reset', () => {
    it('throws ResetNotSupportedError for every level', async () => {
      const lockDir = mkTempDir()
      const deps = makeFakeDeps({ lockDir })
      const c = new TestContainer(deps)

      await expect(c.reset('soft')).rejects.toBeInstanceOf(
        ResetNotSupportedError,
      )
      await expect(c.reset('wipe-agent')).rejects.toBeInstanceOf(
        ResetNotSupportedError,
      )
      await expect(c.reset('hard')).rejects.toBeInstanceOf(
        ResetNotSupportedError,
      )
    })
  })

  describe('path translation', () => {
    it('round-trips host ↔ container paths under a declared mount', () => {
      const lockDir = mkTempDir()
      const deps = makeFakeDeps({ lockDir })
      const c = new TestContainer(deps)

      const host = '/host/root/agents/a/home/file.txt'
      const inContainer = c.toContainerPath(host)
      expect(inContainer).toBe('/data/root/agents/a/home/file.txt')
      expect(c.toHostPath(inContainer)).toBe(host)
    })

    it('rejects host paths outside any declared mount', () => {
      const lockDir = mkTempDir()
      const deps = makeFakeDeps({ lockDir })
      const c = new TestContainer(deps)

      expect(() => c.toContainerPath('/etc/passwd')).toThrow(
        PathOutsideMountsError,
      )
      expect(() => c.toHostPath('/proc/cpuinfo')).toThrow(
        PathOutsideMountsError,
      )
    })

    it('translates the mount root itself (no suffix)', () => {
      const lockDir = mkTempDir()
      const deps = makeFakeDeps({ lockDir })
      const c = new TestContainer(deps)

      expect(c.toContainerPath('/host/root')).toBe('/data/root')
      expect(c.toHostPath('/data/root')).toBe('/host/root')
    })
  })

  describe('subscribeState', () => {
    it('fires every transition and stops after unsubscribe', async () => {
      const lockDir = mkTempDir()
      const deps = makeFakeDeps({ lockDir })
      const c = new TestContainer(deps)
      const transitions: ContainerState[] = []
      const unsubscribe = c.subscribeState((s) => transitions.push(s))

      await c.start()
      expect(transitions.at(-1)).toBe('running')

      unsubscribe()
      await c.stop()
      // No new transitions recorded after unsubscribe.
      expect(transitions.at(-1)).toBe('running')
    })
  })
})
