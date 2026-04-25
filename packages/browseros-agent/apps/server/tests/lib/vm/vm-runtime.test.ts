/**
 * @license
 * Copyright 2025 BrowserOS
 */

import { afterEach, beforeEach, describe, expect, it, mock } from 'bun:test'
import {
  chmod,
  mkdir,
  mkdtemp,
  readFile,
  rm,
  writeFile,
} from 'node:fs/promises'
import { dirname, join } from 'node:path'
import { logger } from '../../../src/lib/logger'
import { VmNotReadyError } from '../../../src/lib/vm/errors'
import type { VmManifest } from '../../../src/lib/vm/manifest'
import {
  getCachedManifestPath,
  getInstalledManifestPath,
  VM_NAME,
} from '../../../src/lib/vm/paths'
import { VM_TELEMETRY_EVENTS } from '../../../src/lib/vm/telemetry'
import { VmRuntime } from '../../../src/lib/vm/vm-runtime'
import { fakeLimactl } from '../../__helpers__/fake-limactl'
import { fakeSsh } from '../../__helpers__/fake-ssh'

const manifest: VmManifest = {
  schemaVersion: 2,
  updatedAt: '2026-04-22T00:00:00.000Z',
  agents: {
    openclaw: {
      image: 'ghcr.io/openclaw/openclaw',
      version: '2026.4.12',
      tarballs: {
        arm64: {
          key: 'vm/images/openclaw-2026.4.12-arm64.tar.gz',
          sha256: 'agent-arm',
          sizeBytes: 1,
        },
        x64: {
          key: 'vm/images/openclaw-2026.4.12-x64.tar.gz',
          sha256: 'agent-x64',
          sizeBytes: 1,
        },
      },
    },
  },
}

describe('VmRuntime', () => {
  let root: string
  let limaHome: string
  let logPath: string
  let templatePath: string

  beforeEach(async () => {
    root = await mkdtemp('/tmp/vmrt-')
    limaHome = join(root, 'lima')
    logPath = join(root, 'limactl.log')
    templatePath = join(root, 'browseros-vm.yaml')
    await writeCachedManifest(root)
    await writeFile(templatePath, 'minimumLimaVersion: 2.0.0\nmounts: []\n')
  })

  afterEach(async () => {
    await rm(root, { recursive: true, force: true })
  })

  it('provisions a fresh VM, waits for rootless nerdctl, and installs the manifest', async () => {
    const limactlPath = await fakeLimactl(
      { list: { stdout: '' }, create: {}, start: {} },
      logPath,
    )
    const sshPath = await prepareReadySsh(limaHome, logPath)
    const runtime = new VmRuntime({
      limactlPath,
      limaHome,
      sshPath,
      templatePath,
      browserosRoot: root,
    })

    await runtime.ensureReady()

    const log = await readFile(logPath, 'utf8')
    expect(log).toContain(`ARGS:create --tty=false --name=${VM_NAME}`)
    expect(log).toContain(`ARGS:start --tty=false ${VM_NAME}`)
    expect(log).toContain(`lima-${VM_NAME} 'nerdctl' 'info'`)
    await expect(
      readFile(getInstalledManifestPath(root), 'utf8'),
    ).resolves.toContain(manifest.updatedAt)
    await expect(
      readFile(join(limaHome, `${VM_NAME}.yaml`), 'utf8'),
    ).resolves.toContain('mountPoint: "/mnt/browseros/vm"')
  })

  it('fills a missing VM cache before reading the cached manifest', async () => {
    await rm(getCachedManifestPath(root), { force: true })
    const limactlPath = await fakeLimactl(
      { list: { stdout: '' }, create: {}, start: {} },
      logPath,
    )
    const sshPath = await prepareReadySsh(limaHome, logPath)
    const ensureCacheAvailable = mock(async () => {
      await writeCachedManifest(root)
    })
    const runtime = new VmRuntime({
      limactlPath,
      limaHome,
      sshPath,
      templatePath,
      browserosRoot: root,
      ensureCacheAvailable,
    })

    await runtime.ensureReady()

    expect(ensureCacheAvailable).toHaveBeenCalledTimes(1)
    await expect(
      readFile(getInstalledManifestPath(root), 'utf8'),
    ).resolves.toContain(manifest.updatedAt)
  })

  it('surfaces cache sync failures before reading a missing manifest', async () => {
    await rm(getCachedManifestPath(root), { force: true })
    const ensureCacheAvailable = mock(async () => {
      throw new Error('cache offline')
    })
    const runtime = new VmRuntime({
      limactlPath: 'unused',
      limaHome,
      browserosRoot: root,
      ensureCacheAvailable,
    })

    await expect(runtime.ensureReady()).rejects.toThrow('cache offline')
    expect(ensureCacheAvailable).toHaveBeenCalledTimes(1)
  })

  it('returns fast when the VM is already running and manifests match', async () => {
    await writeInstalledManifest(root)
    const limactlPath = await fakeLimactl(
      {
        list: {
          stdout: JSON.stringify([
            { name: VM_NAME, status: 'Running', dir: limaHome },
          ]),
        },
        create: { stderr: 'should not create', exit: 9 },
        start: { stderr: 'should not start', exit: 9 },
      },
      logPath,
    )
    const sshPath = await prepareReadySsh(limaHome, logPath)
    const runtime = new VmRuntime({
      limactlPath,
      limaHome,
      sshPath,
      browserosRoot: root,
    })

    await runtime.ensureReady()

    const log = await readFile(logPath, 'utf8')
    expect(log).toContain('ARGS:list --format json')
    expect(log).not.toContain('ARGS:create')
    expect(log).not.toContain('ARGS:start')
  })

  it('starts an existing stopped VM without recreating it', async () => {
    await writeInstalledManifest(root)
    const limactlPath = await fakeLimactl(
      {
        list: {
          stdout: JSON.stringify([
            { name: VM_NAME, status: 'Stopped', dir: limaHome },
          ]),
        },
        start: {},
      },
      logPath,
    )
    const sshPath = await prepareReadySsh(limaHome, logPath)
    const runtime = new VmRuntime({
      limactlPath,
      limaHome,
      sshPath,
      browserosRoot: root,
    })

    await runtime.ensureReady()

    const log = await readFile(logPath, 'utf8')
    expect(log).toContain(`ARGS:start --tty=false ${VM_NAME}`)
    expect(log).not.toContain('ARGS:create')
  })

  it('recreates an existing VM that does not have the containerd runtime marker', async () => {
    await writeInstalledManifest(root)
    const limactlPath = await fakeLimactl(
      {
        list: {
          stdout: JSON.stringify([
            { name: VM_NAME, status: 'Running', dir: limaHome },
          ]),
        },
        stop: {},
        delete: {},
        create: {},
        start: {},
      },
      logPath,
    )
    const sshPath = await fakeRootfulThenReadySsh(root, logPath)
    await writeSshConfig(limaHome)
    const runtime = new VmRuntime({
      limactlPath,
      limaHome,
      sshPath,
      templatePath,
      browserosRoot: root,
    })

    await runtime.ensureReady()

    const log = await readFile(logPath, 'utf8')
    expect(log).toContain(`lima-${VM_NAME} 'nerdctl' 'info'`)
    expect(log).toContain(
      `lima-${VM_NAME} 'sh' '-lc' 'cat /etc/browseros-vm-version 2>/dev/null || true'`,
    )
    expect(log).toContain(`ARGS:stop ${VM_NAME}`)
    expect(log).toContain(`ARGS:delete --force ${VM_NAME}`)
    expect(log).toContain(`ARGS:create --tty=false --name=${VM_NAME}`)
    expect(log).toContain(`ARGS:start --tty=false ${VM_NAME}`)
  })

  it('treats stopVm as idempotent when the VM is already stopped', async () => {
    const limactlPath = await fakeLimactl(
      { stop: { stderr: 'instance is not running', exit: 1 } },
      logPath,
    )
    const runtime = new VmRuntime({
      limactlPath,
      limaHome,
      browserosRoot: root,
    })

    await expect(runtime.stopVm()).resolves.toBeUndefined()
  })

  it('requires a bundled Lima template for fresh VM provisioning', async () => {
    const limactlPath = await fakeLimactl({ list: { stdout: '' } }, logPath)
    const runtime = new VmRuntime({
      limactlPath,
      limaHome,
      browserosRoot: root,
    })

    await expect(runtime.ensureReady()).rejects.toThrow('Lima template path')
  })

  it('throws VmNotReadyError when rootless nerdctl never becomes ready', async () => {
    const limactlPath = await fakeLimactl(
      { list: { stdout: '' }, create: {}, start: {} },
      logPath,
    )
    const sshPath = await prepareFailingSsh(limaHome, logPath)
    const runtime = new VmRuntime({
      limactlPath,
      limaHome,
      sshPath,
      templatePath,
      browserosRoot: root,
      readinessTimeoutMs: 10,
      readinessPollMs: 1,
    })

    await expect(runtime.ensureReady()).rejects.toThrow(VmNotReadyError)
  })

  it('exposes a reset stub with a follow-up-plan message', async () => {
    const limactlPath = await fakeLimactl({}, logPath)
    const runtime = new VmRuntime({
      limactlPath,
      limaHome,
      browserosRoot: root,
    })

    await expect(runtime.reset('bad disk')).rejects.toThrow(
      'VmRuntime.reset is not implemented yet',
    )
  })

  it('logs upgrade mismatch and preserves the installed manifest until upgrade happens', async () => {
    await writeInstalledManifest(root, '2026-04-21T00:00:00.000Z')
    const limactlPath = await fakeLimactl(
      {
        list: {
          stdout: JSON.stringify([
            { name: VM_NAME, status: 'Running', dir: limaHome },
          ]),
        },
      },
      logPath,
    )
    const sshPath = await prepareReadySsh(limaHome, logPath)
    const runtime = new VmRuntime({
      limactlPath,
      limaHome,
      sshPath,
      templatePath,
      browserosRoot: root,
    })
    const originalWarn = logger.warn
    const warnings: Array<{
      message: string
      meta?: Record<string, unknown>
    }> = []
    logger.warn = (message, meta) => warnings.push({ message, meta })

    try {
      await runtime.ensureReady()
    } finally {
      logger.warn = originalWarn
    }

    expect(warnings).toContainEqual({
      message: VM_TELEMETRY_EVENTS.upgradeDetected,
      meta: {
        from: '2026-04-21T00:00:00.000Z',
        to: '2026-04-22T00:00:00.000Z',
      },
    })
    expect(await readInstalledUpdatedAt(root)).toBe('2026-04-21T00:00:00.000Z')
  })

  it('logs downgrade mismatch and preserves a newer installed manifest', async () => {
    await writeInstalledManifest(root, '2026-04-23T00:00:00.000Z')
    const limactlPath = await fakeLimactl(
      {
        list: {
          stdout: JSON.stringify([
            { name: VM_NAME, status: 'Running', dir: limaHome },
          ]),
        },
      },
      logPath,
    )
    const sshPath = await prepareReadySsh(limaHome, logPath)
    const runtime = new VmRuntime({
      limactlPath,
      limaHome,
      sshPath,
      templatePath,
      browserosRoot: root,
    })
    const originalWarn = logger.warn
    const warnings: Array<{
      message: string
      meta?: Record<string, unknown>
    }> = []
    logger.warn = (message, meta) => warnings.push({ message, meta })

    try {
      await runtime.ensureReady()
    } finally {
      logger.warn = originalWarn
    }

    expect(warnings).toContainEqual({
      message: VM_TELEMETRY_EVENTS.downgradeDetected,
      meta: {
        from: '2026-04-23T00:00:00.000Z',
        to: '2026-04-22T00:00:00.000Z',
      },
    })
    expect(await readInstalledUpdatedAt(root)).toBe('2026-04-23T00:00:00.000Z')
  })

  it('does not auto-reset when rootless nerdctl readiness fails', async () => {
    const limactlPath = await fakeLimactl(
      { list: { stdout: '' }, create: {}, start: {} },
      logPath,
    )
    const sshPath = await prepareFailingSsh(limaHome, logPath)
    const runtime = new VmRuntime({
      limactlPath,
      limaHome,
      sshPath,
      templatePath,
      browserosRoot: root,
      readinessTimeoutMs: 10,
      readinessPollMs: 1,
    })
    let resetCalled = false
    runtime.reset = async () => {
      resetCalled = true
      throw new Error('reset called')
    }

    await expect(runtime.ensureReady()).rejects.toThrow(VmNotReadyError)
    expect(resetCalled).toBe(false)
  })

  it('delegates runCommand through ssh', async () => {
    const sshPath = await fakeSsh({}, logPath)
    const sshConfig = join(limaHome, VM_NAME, 'ssh.config')
    await mkdir(join(limaHome, VM_NAME), { recursive: true })
    await writeFile(sshConfig, '')
    const runtime = new VmRuntime({
      limactlPath: 'unused',
      limaHome,
      sshPath,
      browserosRoot: root,
    })

    await expect(runtime.runCommand(['nerdctl', 'version'])).resolves.toBe(0)

    const log = await readFile(logPath, 'utf8')
    expect(log).toContain(
      `ARGS:-F ${sshConfig} lima-${VM_NAME} 'nerdctl' 'version'`,
    )
  })

  it('resolves and caches the VM default gateway through ssh', async () => {
    const sshPath = await fakeSsh(
      {
        stdout:
          'default via 192.168.5.2 dev eth0 proto dhcp src 192.168.5.15 metric 100\n',
      },
      logPath,
    )
    const sshConfig = join(limaHome, VM_NAME, 'ssh.config')
    await mkdir(join(limaHome, VM_NAME), { recursive: true })
    await writeFile(sshConfig, '')
    const runtime = new VmRuntime({
      limactlPath: 'unused',
      limaHome,
      sshPath,
      browserosRoot: root,
    })

    await expect(runtime.getDefaultGateway()).resolves.toBe('192.168.5.2')
    await expect(runtime.getDefaultGateway()).resolves.toBe('192.168.5.2')

    const log = await readFile(logPath, 'utf8')
    expect(log.match(/'ip' '-4' 'route' 'show' 'default'/g)).toHaveLength(1)
  })
})

async function writeCachedManifest(root: string): Promise<void> {
  const manifestPath = getCachedManifestPath(root)
  await mkdir(dirname(manifestPath), { recursive: true })
  await writeFile(manifestPath, `${JSON.stringify(manifest)}\n`)
}

async function writeInstalledManifest(
  root: string,
  updatedAt = manifest.updatedAt,
): Promise<void> {
  const manifestPath = getInstalledManifestPath(root)
  await mkdir(dirname(manifestPath), { recursive: true })
  await writeFile(
    manifestPath,
    `${JSON.stringify({ ...manifest, updatedAt })}\n`,
  )
}

async function readInstalledUpdatedAt(root: string): Promise<string> {
  const raw = await readFile(getInstalledManifestPath(root), 'utf8')
  return (JSON.parse(raw) as VmManifest).updatedAt
}

async function prepareReadySsh(
  limaHome: string,
  logPath: string,
): Promise<string> {
  await writeSshConfig(limaHome)
  return fakeSsh({}, logPath)
}

async function prepareFailingSsh(
  limaHome: string,
  logPath: string,
): Promise<string> {
  await writeSshConfig(limaHome)
  return fakeSsh(
    {
      stderr:
        'rootless containerd not running? stat /run/user/501/containerd-rootless: no such file or directory',
      exit: 1,
    },
    logPath,
  )
}

async function writeSshConfig(limaHome: string): Promise<void> {
  await mkdir(join(limaHome, VM_NAME), { recursive: true })
  await writeFile(join(limaHome, VM_NAME, 'ssh.config'), '')
}

async function fakeRootfulThenReadySsh(
  root: string,
  logPath: string,
): Promise<string> {
  const path = join(root, 'ssh-rootful-then-ready')
  const counterPath = join(root, 'ssh-rootful-then-ready.count')
  const body = `#!/usr/bin/env bash
set -u
echo "ARGS:$*" >> "${logPath}"
count="$(cat "${counterPath}" 2>/dev/null || echo 0)"
next=$((count + 1))
printf '%s' "$next" > "${counterPath}"
case "$count" in
  0)
    echo "rootless containerd not running" >&2
    exit 1
    ;;
  1)
    printf 'runtime:containerd\\n'
    exit 0
    ;;
  *)
    exit 0
    ;;
esac
`
  await writeFile(path, body)
  await chmod(path, 0o755)
  return path
}
