/**
 * @license
 * Copyright 2025 BrowserOS
 */

import { describe, expect, it } from 'bun:test'
import {
  ActionNotSupportedError,
  ContainerAgentRuntime,
  type RuntimeCapability,
  type RuntimeStatusSnapshot,
} from '../../../../src/lib/agents/runtime'
import type {
  ManagedContainerDeps,
  MountRoot,
} from '../../../../src/lib/container/managed'
import type { ContainerSpec } from '../../../../src/lib/container/types'

interface Call {
  kind: 'install' | 'start' | 'stop' | 'restart' | 'reset'
  args?: unknown
}

class TestRuntime extends ContainerAgentRuntime {
  readonly descriptor = {
    adapterId: 'test',
    displayName: 'Test',
    kind: 'container' as const,
    defaultImage: 'docker.io/test:latest',
    containerName: 'test-container',
    platforms: ['darwin' as NodeJS.Platform],
  }

  calls: Call[] = []
  capabilities: ReadonlyArray<RuntimeCapability> | null = null

  getPerAgentHomeDir(agentId: string): string {
    return `/tmp/test/${agentId}`
  }

  override getCapabilities(): ReadonlyArray<RuntimeCapability> {
    return this.capabilities ?? super.getCapabilities()
  }

  protected mountRoots(): readonly MountRoot[] {
    return []
  }

  protected async buildContainerSpec(): Promise<ContainerSpec> {
    return {
      name: this.descriptor.containerName,
      image: this.descriptor.defaultImage,
    }
  }

  protected async readinessProbe(): Promise<boolean> {
    return true
  }

  override async install(_opts: { onLog?: (m: string) => void } = {}) {
    this.calls.push({ kind: 'install' })
  }
  override async start(_opts: { onLog?: (m: string) => void } = {}) {
    this.calls.push({ kind: 'start' })
  }
  override async stop() {
    this.calls.push({ kind: 'stop' })
  }
  override async restart(_opts: { onLog?: (m: string) => void } = {}) {
    this.calls.push({ kind: 'restart' })
  }
  override async reset(level: 'soft' | 'wipe-agent' | 'hard', opts: unknown) {
    this.calls.push({ kind: 'reset', args: { level, opts } })
  }
}

function makeDeps(): ManagedContainerDeps {
  return {
    cli: {} as ManagedContainerDeps['cli'],
    loader: {} as ManagedContainerDeps['loader'],
    vm: {} as ManagedContainerDeps['vm'],
    limactlPath: '/opt/homebrew/bin/limactl',
    limaHome: '/tmp/lima',
    vmName: 'browseros-vm',
    lockDir: '/tmp/locks',
  }
}

describe('ContainerAgentRuntime', () => {
  it('default capabilities cover lifecycle + reset levels + logs', () => {
    const r = new TestRuntime(makeDeps())
    expect(r.getCapabilities()).toEqual([
      'install',
      'start',
      'stop',
      'restart',
      'reset-soft',
      'reset-wipe-agent',
      'reset-hard',
      'logs',
    ])
  })

  it('getStatusSnapshot maps state, isReady, containerName', () => {
    const r = new TestRuntime(makeDeps())
    const snap = r.getStatusSnapshot()
    expect(snap.adapterId).toBe('test')
    expect(snap.containerName).toBe('test-container')
    expect(snap.state).toBe('not_installed')
    expect(snap.isReady).toBe(false)
    expect(snap.lastError).toBeNull()
  })

  it('isReady is true only when state is running', () => {
    const r = new TestRuntime(makeDeps())
    // biome-ignore lint/complexity/useLiteralKeys: protected access
    r['state'] = 'running'
    expect(r.getStatusSnapshot().isReady).toBe(true)
    // biome-ignore lint/complexity/useLiteralKeys: protected access
    r['state'] = 'errored'
    expect(r.getStatusSnapshot().isReady).toBe(false)
  })

  it('subscribe wires through subscribeState and emits a snapshot', () => {
    const r = new TestRuntime(makeDeps())
    const seen: RuntimeStatusSnapshot[] = []
    const off = r.subscribe((s) => seen.push(s))
    // biome-ignore lint/complexity/useLiteralKeys: protected access
    r['setState']('starting')
    // biome-ignore lint/complexity/useLiteralKeys: protected access
    r['setState']('running')
    off()
    // biome-ignore lint/complexity/useLiteralKeys: protected access
    r['setState']('stopped')
    expect(seen.map((s) => s.state)).toEqual(['starting', 'running'])
    expect(seen[1].isReady).toBe(true)
  })

  describe('executeAction dispatch', () => {
    it('routes install/start/stop/restart to inherited methods', async () => {
      const r = new TestRuntime(makeDeps())
      await r.executeAction({ type: 'install' })
      await r.executeAction({ type: 'start' })
      await r.executeAction({ type: 'stop' })
      await r.executeAction({ type: 'restart' })
      expect(r.calls.map((c) => c.kind)).toEqual([
        'install',
        'start',
        'stop',
        'restart',
      ])
    })

    it('routes reset variants with correct level + agentId', async () => {
      const r = new TestRuntime(makeDeps())
      await r.executeAction({ type: 'reset-soft' })
      await r.executeAction({ type: 'reset-wipe-agent', agentId: 'agent-7' })
      await r.executeAction({ type: 'reset-hard' })
      const resets = r.calls.filter((c) => c.kind === 'reset')
      expect(resets).toHaveLength(3)
      expect((resets[0].args as { level: string }).level).toBe('soft')
      expect(
        (resets[1].args as { level: string; opts: { agentId: string } }).opts
          .agentId,
      ).toBe('agent-7')
      expect((resets[2].args as { level: string }).level).toBe('hard')
    })

    it('throws ActionNotSupportedError when capability is filtered out', async () => {
      const r = new TestRuntime(makeDeps())
      r.capabilities = ['start', 'stop']
      await expect(r.executeAction({ type: 'install' })).rejects.toBeInstanceOf(
        ActionNotSupportedError,
      )
      expect(r.calls).toEqual([])
    })

    it('throws ActionNotSupportedError for host-only actions', async () => {
      const r = new TestRuntime(makeDeps())
      await expect(
        r.executeAction({ type: 'reinstall-cli' }),
      ).rejects.toBeInstanceOf(ActionNotSupportedError)
      await expect(
        r.executeAction({ type: 'check-auth' }),
      ).rejects.toBeInstanceOf(ActionNotSupportedError)
    })
  })
})
