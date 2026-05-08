/**
 * @license
 * Copyright 2025 BrowserOS
 */

import { describe, expect, it, mock } from 'bun:test'
import {
  ActionNotSupportedError,
  HostProcessAgentRuntime,
  type HostProcessAgentRuntimeDeps,
  type RuntimeStatusSnapshot,
} from '../../../../src/lib/agents/runtime'

class TestRuntime extends HostProcessAgentRuntime {
  readonly descriptor = {
    adapterId: 'host-test',
    displayName: 'Host Test',
    kind: 'host-process' as const,
    platforms: ['darwin' as NodeJS.Platform],
  }

  reinstallCalls = 0
  authCalls = 0

  getPerAgentHomeDir(agentId: string): string {
    return `/tmp/host-test/${agentId}`
  }

  protected override async handleReinstallCli(): Promise<void> {
    this.reinstallCalls += 1
  }

  protected override async checkAuth(): Promise<void> {
    this.authCalls += 1
  }
}

function makeRuntime(
  overrides: Partial<HostProcessAgentRuntimeDeps> = {},
): TestRuntime {
  return new TestRuntime({
    binaryName: 'fake-cli',
    spawnProbe: async () => ({ exitCode: 0, stdout: '1.2.3\n', stderr: '' }),
    ...overrides,
  })
}

describe('HostProcessAgentRuntime', () => {
  it('starts in cli_missing state', () => {
    const r = makeRuntime()
    const snap = r.getStatusSnapshot()
    expect(snap.state).toBe('cli_missing')
    expect(snap.isReady).toBe(false)
    expect(snap.details).toEqual({ binaryVersion: null })
  })

  it('default capabilities are reinstall-cli + check-auth', () => {
    expect(makeRuntime().getCapabilities()).toEqual([
      'reinstall-cli',
      'check-auth',
    ])
  })

  describe('probeHealth', () => {
    it('transitions to cli_present on exit 0 and records version', async () => {
      const spawnProbe = mock(async () => ({
        exitCode: 0,
        stdout: '1.2.3\n',
        stderr: '',
      }))
      const r = makeRuntime({ spawnProbe })
      await r.probeHealth()
      const snap = r.getStatusSnapshot()
      expect(snap.state).toBe('cli_present')
      expect(snap.isReady).toBe(true)
      expect(snap.details?.binaryVersion).toBe('1.2.3')
      expect(spawnProbe).toHaveBeenCalledTimes(1)
    })

    it('transitions to cli_unhealthy on non-zero exit', async () => {
      const r = makeRuntime({
        spawnProbe: async () => ({
          exitCode: 1,
          stdout: '',
          stderr: 'broken',
        }),
      })
      await r.probeHealth()
      const snap = r.getStatusSnapshot()
      expect(snap.state).toBe('cli_unhealthy')
      expect(snap.lastError).toMatch(/exited 1/)
      expect(snap.lastError).toMatch(/broken/)
    })

    it('transitions to cli_missing when spawn throws', async () => {
      const r = makeRuntime({
        spawnProbe: async () => {
          throw new Error('ENOENT')
        },
      })
      await r.probeHealth()
      const snap = r.getStatusSnapshot()
      expect(snap.state).toBe('cli_missing')
      expect(snap.lastError).toBe('ENOENT')
    })

    it('does not stamp the cache when probe throws (lets next call retry)', async () => {
      let attempt = 0
      const spawnProbe = mock(async () => {
        attempt += 1
        if (attempt === 1) throw new Error('ENOENT')
        return { exitCode: 0, stdout: 'ok', stderr: '' }
      })
      const r = makeRuntime({ spawnProbe, probeCacheMs: 60_000 })
      await r.probeHealth()
      // No force flag — should still re-probe because the first call
      // failed and must not have advanced the cache.
      await r.probeHealth()
      expect(spawnProbe).toHaveBeenCalledTimes(2)
      expect(r.getStatusSnapshot().state).toBe('cli_present')
    })

    it('caches probe results within the cache window', async () => {
      const spawnProbe = mock(async () => ({
        exitCode: 0,
        stdout: 'v',
        stderr: '',
      }))
      const r = makeRuntime({ spawnProbe, probeCacheMs: 60_000 })
      await r.probeHealth()
      await r.probeHealth()
      await r.probeHealth()
      expect(spawnProbe).toHaveBeenCalledTimes(1)
    })

    it('force=true bypasses the cache', async () => {
      const spawnProbe = mock(async () => ({
        exitCode: 0,
        stdout: 'v',
        stderr: '',
      }))
      const r = makeRuntime({ spawnProbe, probeCacheMs: 60_000 })
      await r.probeHealth()
      await r.probeHealth(true)
      expect(spawnProbe).toHaveBeenCalledTimes(2)
    })

    it('uses versionProbeArgs override when provided', async () => {
      const spawnProbe = mock(async () => ({
        exitCode: 0,
        stdout: 'v',
        stderr: '',
      }))
      const r = makeRuntime({
        spawnProbe,
        versionProbeArgs: ['custom-bin', '-V'],
      })
      await r.probeHealth()
      expect(spawnProbe.mock.calls[0]?.[0]).toEqual(['custom-bin', '-V'])
    })
  })

  describe('subscribe', () => {
    it('fires listener on state changes only', async () => {
      const seen: RuntimeStatusSnapshot[] = []
      const r = makeRuntime()
      r.subscribe((s) => seen.push(s))
      await r.probeHealth()
      await r.probeHealth(true)
      // First probe: cli_missing → cli_present (one fire). Second
      // probe: stays cli_present, no fire.
      expect(seen.map((s) => s.state)).toEqual(['cli_present'])
    })

    it('unsubscribe stops further notifications', async () => {
      const seen: RuntimeStatusSnapshot[] = []
      const r = makeRuntime()
      const off = r.subscribe((s) => seen.push(s))
      off()
      await r.probeHealth()
      expect(seen).toEqual([])
    })
  })

  describe('executeAction', () => {
    it('routes reinstall-cli + check-auth to subclass hooks', async () => {
      const r = makeRuntime()
      await r.executeAction({ type: 'reinstall-cli' })
      await r.executeAction({ type: 'check-auth' })
      expect(r.reinstallCalls).toBe(1)
      expect(r.authCalls).toBe(1)
    })

    it('throws ActionNotSupportedError for container-only actions', async () => {
      const r = makeRuntime()
      await expect(r.executeAction({ type: 'install' })).rejects.toBeInstanceOf(
        ActionNotSupportedError,
      )
      await expect(r.executeAction({ type: 'start' })).rejects.toBeInstanceOf(
        ActionNotSupportedError,
      )
      await expect(
        r.executeAction({ type: 'reset-soft' }),
      ).rejects.toBeInstanceOf(ActionNotSupportedError)
    })

    it('gates on getCapabilities() — subclass-filtered actions throw', async () => {
      class FilteredRuntime extends TestRuntime {
        override getCapabilities() {
          return ['check-auth' as const]
        }
      }
      const r = new FilteredRuntime({ binaryName: 'fake-cli' })
      await expect(
        r.executeAction({ type: 'reinstall-cli' }),
      ).rejects.toBeInstanceOf(ActionNotSupportedError)
      expect(r.reinstallCalls).toBe(0)
      // Whitelisted action still goes through.
      await r.executeAction({ type: 'check-auth' })
      expect(r.authCalls).toBe(1)
    })

    it('default handleReinstallCli throws if subclass does not override', async () => {
      class BareRuntime extends HostProcessAgentRuntime {
        readonly descriptor = {
          adapterId: 'bare',
          displayName: 'Bare',
          kind: 'host-process' as const,
          platforms: ['darwin' as NodeJS.Platform],
        }
        getPerAgentHomeDir() {
          return '/tmp/bare'
        }
      }
      const r = new BareRuntime({ binaryName: 'bare-cli' })
      await expect(r.executeAction({ type: 'reinstall-cli' })).rejects.toThrow(
        /not installed/,
      )
    })
  })

  describe('buildExecArgv', () => {
    it('joins argv with no env prefix when env is empty', () => {
      const r = makeRuntime()
      const out = r.buildExecArgv({ argv: ['fake-cli', '--help'] })
      expect(out).toBe('fake-cli --help')
    })

    it('emits an env prefix when env is set', () => {
      const r = makeRuntime()
      const out = r.buildExecArgv({
        argv: ['fake-cli', 'run'],
        env: { AGENT_HOME: '/tmp/h', LOG: '1' },
      })
      expect(out).toBe('env AGENT_HOME=/tmp/h LOG=1 fake-cli run')
    })
  })
})
