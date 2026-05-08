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

  describe('isImageCurrent', () => {
    function attachImageRef(
      deps: ReturnType<typeof makeFakeDeps>,
      ref: string | null,
    ): void {
      // biome-ignore lint/suspicious/noExplicitAny: extending the fake at runtime
      ;(deps.cli as any).containerImageRef = async () => ref
    }

    it('returns true when ref matches descriptor.defaultImage', async () => {
      const deps = makeFakeDeps({ lockDir: mkTempDir() })
      attachImageRef(deps, 'docker.io/test:latest')
      expect(await new TestContainer(deps).isImageCurrent()).toBe(true)
    })

    it('returns true for SHA-pinned variants of the expected ref', async () => {
      const deps = makeFakeDeps({ lockDir: mkTempDir() })
      attachImageRef(deps, 'docker.io/test:latest@sha256:deadbeef')
      expect(await new TestContainer(deps).isImageCurrent()).toBe(true)
    })

    it('returns false when ref differs', async () => {
      const deps = makeFakeDeps({ lockDir: mkTempDir() })
      attachImageRef(deps, 'docker.io/test:older')
      expect(await new TestContainer(deps).isImageCurrent()).toBe(false)
    })

    it('returns false when the container is missing', async () => {
      const deps = makeFakeDeps({ lockDir: mkTempDir() })
      attachImageRef(deps, null)
      expect(await new TestContainer(deps).isImageCurrent()).toBe(false)
    })
  })

  describe('getLogs / tailLogs', () => {
    it('getLogs collects lines from cli.runCommand', async () => {
      const deps = makeFakeDeps({ lockDir: mkTempDir() })
      let captured: string[] = []
      // biome-ignore lint/suspicious/noExplicitAny: extending the fake at runtime
      ;(deps.cli as any).runCommand = async (
        args: string[],
        onLog?: (line: string) => void,
      ) => {
        captured = args
        onLog?.('line-a')
        onLog?.('line-b')
        return { exitCode: 0, stdout: '', stderr: '' }
      }
      const c = new TestContainer(deps)
      const lines = await c.getLogs(120)
      expect(lines).toEqual(['line-a', 'line-b'])
      expect(captured).toEqual(['logs', '-n', '120', 'test-container'])
    })

    it('getLogs returns [] when the container does not exist', async () => {
      const deps = makeFakeDeps({ lockDir: mkTempDir() })
      // biome-ignore lint/suspicious/noExplicitAny: extending the fake at runtime
      ;(deps.cli as any).runCommand = async (
        _args: string[],
        onLog?: (line: string) => void,
      ) => {
        onLog?.('Error: no such container: test-container')
        return {
          exitCode: 1,
          stdout: '',
          stderr: 'Error: no such container: test-container',
        }
      }
      const lines = await new TestContainer(deps).getLogs()
      expect(lines).toEqual([])
    })

    it('getLogs throws on non-zero exit that is not a no-such-container error', async () => {
      const deps = makeFakeDeps({ lockDir: mkTempDir() })
      // biome-ignore lint/suspicious/noExplicitAny: extending the fake at runtime
      ;(deps.cli as any).runCommand = async () => ({
        exitCode: 2,
        stdout: '',
        stderr: 'permission denied',
      })
      await expect(new TestContainer(deps).getLogs()).rejects.toThrow(
        /exited 2.*permission denied/,
      )
    })

    it('tailLogs returns the unsubscribe handle from cli.tailLogs', () => {
      const deps = makeFakeDeps({ lockDir: mkTempDir() })
      let unsubscribed = false
      let receivedName: string | null = null
      // biome-ignore lint/suspicious/noExplicitAny: extending the fake at runtime
      ;(deps.cli as any).tailLogs = (name: string, _onLine: unknown) => {
        receivedName = name
        return () => {
          unsubscribed = true
        }
      }
      const c = new TestContainer(deps)
      const stop = c.tailLogs(() => {})
      expect(receivedName).toBe('test-container')
      stop()
      expect(unsubscribed).toBe(true)
    })
  })

  describe('runOneShot', () => {
    type OneShotFakes = {
      removed: string[]
      created: ContainerSpec[]
      runCalls: string[][]
      runResult: { exitCode: number; stdout: string; stderr: string }
    }

    function attachOneShotFakes(
      deps: ReturnType<typeof makeFakeDeps>,
    ): OneShotFakes {
      const state: OneShotFakes = {
        removed: [],
        created: [],
        runCalls: [],
        runResult: { exitCode: 0, stdout: 'hi', stderr: '' },
      }
      const cli = deps.cli as unknown as Record<string, unknown>
      cli.removeContainer = async (
        name: string,
        _opts?: { force?: boolean },
      ) => {
        state.removed.push(name)
      }
      cli.waitForContainerNameRelease = async () => {}
      cli.createContainer = async (spec: ContainerSpec) => {
        state.created.push(spec)
      }
      cli.runCommand = async (args: string[]) => {
        state.runCalls.push(args)
        return state.runResult
      }
      return state
    }

    it('creates a sibling -setup container with no ports/health and the requested argv', async () => {
      const deps = makeFakeDeps({ lockDir: mkTempDir() })
      const fakes = attachOneShotFakes(deps)
      const c = new TestContainer(deps)

      const result = await c.runOneShot(['echo', 'hello'], {
        env: { EXTRA: '1' },
      })

      expect(result).toEqual({ exitCode: 0, stdout: 'hi', stderr: '' })
      expect(fakes.created).toHaveLength(1)
      const setupSpec = fakes.created[0]
      expect(setupSpec.name).toBe('test-container-setup')
      expect(setupSpec.image).toBe('docker.io/test:latest')
      expect(setupSpec.restart).toBe('no')
      expect(setupSpec.ports).toBeUndefined()
      expect(setupSpec.health).toBeUndefined()
      expect(setupSpec.command).toEqual(['echo', 'hello'])
      expect(setupSpec.env).toEqual({ FOO: 'bar', EXTRA: '1' })
      expect(fakes.runCalls).toEqual([['start', '-a', 'test-container-setup']])
      expect(
        fakes.removed.filter((n) => n === 'test-container-setup'),
      ).toHaveLength(2)
    })

    it('force-removes the sibling even when the inner command throws', async () => {
      const deps = makeFakeDeps({ lockDir: mkTempDir() })
      const fakes = attachOneShotFakes(deps)
      const cli = deps.cli as unknown as Record<string, unknown>
      cli.runCommand = async () => {
        throw new Error('boom')
      }
      const c = new TestContainer(deps)
      await expect(c.runOneShot(['noop'])).rejects.toThrow(/boom/)
      expect(
        fakes.removed.filter((n) => n === 'test-container-setup'),
      ).toHaveLength(2)
    })

    it('drops onLog calls fired by the underlying runCommand after a timeout', async () => {
      const deps = makeFakeDeps({ lockDir: mkTempDir() })
      attachOneShotFakes(deps)
      const cli = deps.cli as unknown as Record<string, unknown>
      let capturedOnLog: ((line: string) => void) | undefined
      cli.runCommand = async (
        _args: string[],
        onLog?: (line: string) => void,
      ) => {
        capturedOnLog = onLog
        await new Promise((resolve) => setTimeout(resolve, 100))
        return { exitCode: 0, stdout: '', stderr: '' }
      }
      const seen: string[] = []
      const c = new TestContainer(deps)
      await expect(
        c.runOneShot(['noop'], {
          processTimeoutMs: 5,
          onLog: (line) => seen.push(line),
        }),
      ).rejects.toThrow(/exceeded timeout/)
      capturedOnLog?.('post-timeout-line')
      expect(seen).toEqual([])
    })

    it('retries createContainer on ContainerNameInUseError', async () => {
      const deps = makeFakeDeps({ lockDir: mkTempDir() })
      const fakes = attachOneShotFakes(deps)
      const cli = deps.cli as unknown as Record<string, unknown>
      let createAttempts = 0
      cli.createContainer = async (spec: ContainerSpec) => {
        createAttempts += 1
        if (createAttempts < 2) {
          const { ContainerNameInUseError } = await import(
            '../../../../src/lib/vm/errors'
          )
          throw new ContainerNameInUseError(
            spec.name,
            'nerdctl create',
            1,
            `container name "${spec.name}" is already used`,
          )
        }
        fakes.created.push(spec)
      }
      const c = new TestContainer(deps)
      await c.runOneShot(['echo'])
      expect(createAttempts).toBe(2)
      expect(fakes.created).toHaveLength(1)
    })
  })
})
