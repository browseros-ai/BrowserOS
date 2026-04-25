/**
 * @license
 * Copyright 2025 BrowserOS
 */

import { afterEach, describe, expect, it } from 'bun:test'
import { chmod, mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises'
import { homedir, tmpdir } from 'node:os'
import { dirname, join } from 'node:path'
import { PATHS } from '@browseros/shared/constants/paths'
import {
  getLegacyOpenClawDir,
  getOpenClawDir,
} from '../../../src/lib/browseros-dir'
import {
  detectArch,
  getCachedManifestPath,
  getContainerdSocketPath,
  getImageCacheDir,
  getInstalledManifestPath,
  getLimaHomeDir,
  getVmCacheDir,
  getVmStateDir,
  hostPathToGuest,
  resolveBundledLimactl,
  resolveBundledLimaTemplate,
} from '../../../src/lib/vm/paths'

describe('VM paths', () => {
  const originalNodeEnv = process.env.NODE_ENV
  const originalPath = process.env.PATH

  afterEach(() => {
    if (originalNodeEnv === undefined) {
      delete process.env.NODE_ENV
    } else {
      process.env.NODE_ENV = originalNodeEnv
    }
    if (originalPath === undefined) {
      delete process.env.PATH
    } else {
      process.env.PATH = originalPath
    }
  })

  it('uses production VM directories below .browseros', () => {
    process.env.NODE_ENV = 'production'

    expect(getLimaHomeDir()).toBe(join(homedir(), '.browseros', 'lima'))
    expect(getVmStateDir()).toBe(join(homedir(), '.browseros', 'vm'))
    expect(getOpenClawDir()).toBe(
      join(homedir(), '.browseros', 'vm', 'openclaw'),
    )
  })

  it('uses development VM directories below .browseros-dev', () => {
    process.env.NODE_ENV = 'development'

    expect(getLimaHomeDir()).toBe(join(homedir(), '.browseros-dev', 'lima'))
    expect(getVmStateDir()).toBe(join(homedir(), '.browseros-dev', 'vm'))
    expect(getOpenClawDir()).toBe(
      join(homedir(), '.browseros-dev', 'vm', 'openclaw'),
    )
  })

  it('keeps the legacy OpenClaw directory addressable for migration', () => {
    process.env.NODE_ENV = 'production'

    expect(getLegacyOpenClawDir()).toBe(
      join(homedir(), PATHS.BROWSEROS_DIR_NAME, PATHS.OPENCLAW_DIR_NAME),
    )
  })

  it('builds cached and installed manifest paths', () => {
    const root = '/Users/foo/.browseros'

    expect(getVmCacheDir(root)).toBe('/Users/foo/.browseros/cache/vm')
    expect(getImageCacheDir(root)).toBe('/Users/foo/.browseros/cache/vm/images')
    expect(getCachedManifestPath(root)).toBe(
      '/Users/foo/.browseros/cache/vm/manifest.json',
    )
    expect(getInstalledManifestPath(root)).toBe(
      '/Users/foo/.browseros/vm/manifest.json',
    )
    expect(getContainerdSocketPath(root)).toBe(
      '/Users/foo/.browseros/lima/browseros-vm/sock/containerd.sock',
    )
  })

  it('translates mounted host paths into guest paths', () => {
    const root = '/Users/foo/.browseros'

    expect(hostPathToGuest('/Users/foo/.browseros/vm/openclaw/x', root)).toBe(
      '/mnt/browseros/vm/openclaw/x',
    )
    expect(
      hostPathToGuest('/Users/foo/.browseros/cache/vm/images/a.tar.gz', root),
    ).toBe('/mnt/browseros/cache/images/a.tar.gz')
  })

  it('rejects unmapped host paths', () => {
    expect(() =>
      hostPathToGuest('/tmp/other', '/Users/foo/.browseros'),
    ).toThrow('not under any known guest mount')
  })

  it('detects supported host architectures', () => {
    expect(detectArch('arm64')).toBe('arm64')
    expect(detectArch('x64')).toBe('x64')
  })

  it('rejects unsupported host architectures', () => {
    expect(() => detectArch('ppc64' as NodeJS.Architecture)).toThrow(
      'unsupported host arch',
    )
  })

  it('resolves the bundled limactl executable', async () => {
    process.env.NODE_ENV = 'production'
    const resourcesDir = await mkdtemp(join(tmpdir(), 'limactl-resources-'))
    const limactlPath = join(
      resourcesDir,
      'bin',
      'third_party',
      'lima',
      'bin',
      'limactl',
    )
    const armGuestAgentPath = join(
      resourcesDir,
      'bin',
      'third_party',
      'lima',
      'share',
      'lima',
      'lima-guestagent.Linux-aarch64.gz',
    )
    const x64GuestAgentPath = join(
      resourcesDir,
      'bin',
      'third_party',
      'lima',
      'share',
      'lima',
      'lima-guestagent.Linux-x86_64.gz',
    )
    await mkdir(dirname(limactlPath), { recursive: true })
    await mkdir(dirname(armGuestAgentPath), { recursive: true })
    await writeFile(limactlPath, '#!/bin/sh\n')
    await writeFile(armGuestAgentPath, 'guest-agent\n')
    await writeFile(x64GuestAgentPath, 'guest-agent\n')

    try {
      expect(resolveBundledLimactl(resourcesDir)).toBe(limactlPath)
    } finally {
      await rm(resourcesDir, { recursive: true, force: true })
    }
  })

  it('validates the x64 bundled Lima guest agent path', async () => {
    process.env.NODE_ENV = 'production'
    const resourcesDir = await mkdtemp(join(tmpdir(), 'limactl-x64-resources-'))
    const limactlPath = join(
      resourcesDir,
      'bin',
      'third_party',
      'lima',
      'bin',
      'limactl',
    )
    const guestAgentPath = join(
      resourcesDir,
      'bin',
      'third_party',
      'lima',
      'share',
      'lima',
      'lima-guestagent.Linux-x86_64.gz',
    )
    await mkdir(dirname(limactlPath), { recursive: true })
    await mkdir(dirname(guestAgentPath), { recursive: true })
    await writeFile(limactlPath, '#!/bin/sh\n')
    await writeFile(guestAgentPath, 'guest-agent\n')

    try {
      expect(resolveBundledLimactl(resourcesDir, 'x64')).toBe(limactlPath)
    } finally {
      await rm(resourcesDir, { recursive: true, force: true })
    }
  })

  it('throws with a runtime packaging hint when the bundled Lima guest agent is missing', async () => {
    process.env.NODE_ENV = 'production'
    const resourcesDir = await mkdtemp(
      join(tmpdir(), 'missing-lima-guest-agent-'),
    )
    const limactlPath = join(
      resourcesDir,
      'bin',
      'third_party',
      'lima',
      'bin',
      'limactl',
    )
    await mkdir(dirname(limactlPath), { recursive: true })
    await writeFile(limactlPath, '#!/bin/sh\n')

    try {
      expect(() => resolveBundledLimactl(resourcesDir)).toThrow(
        'bundled Lima guest agent not found',
      )
    } finally {
      await rm(resourcesDir, { recursive: true, force: true })
    }
  })

  it('uses PATH limactl in development mode', async () => {
    process.env.NODE_ENV = 'development'
    const binDir = await createFakeLimactlPath()

    try {
      expect(resolveBundledLimactl('/tmp/missing-dev-resources')).toBe(
        join(binDir, 'limactl'),
      )
    } finally {
      await rm(binDir, { recursive: true, force: true })
    }
  })

  it('uses PATH limactl in test mode', async () => {
    process.env.NODE_ENV = 'test'
    const binDir = await createFakeLimactlPath()

    try {
      expect(resolveBundledLimactl('/tmp/missing-test-resources')).toBe(
        join(binDir, 'limactl'),
      )
    } finally {
      await rm(binDir, { recursive: true, force: true })
    }
  })

  it('throws with a brew install hint when host limactl is missing', async () => {
    process.env.NODE_ENV = 'development'
    const binDir = await mkdtemp(join(tmpdir(), 'missing-host-limactl-'))
    process.env.PATH = binDir

    try {
      expect(() => resolveBundledLimactl('/tmp/missing-dev-resources')).toThrow(
        'brew install lima',
      )
    } finally {
      await rm(binDir, { recursive: true, force: true })
    }
  })

  it('throws with a build-tools hint when bundled limactl is missing', () => {
    process.env.NODE_ENV = 'production'

    expect(() => resolveBundledLimactl('/tmp/missing-resources')).toThrow(
      'build-tools README',
    )
  })

  it('resolves the bundled Lima template', async () => {
    process.env.NODE_ENV = 'production'
    const resourcesDir = await mkdtemp(join(tmpdir(), 'lima-template-'))
    const templatePath = join(resourcesDir, 'vm', 'browseros-vm.yaml')
    await mkdir(dirname(templatePath), { recursive: true })
    await writeFile(templatePath, 'mounts: []\n')

    try {
      expect(resolveBundledLimaTemplate(resourcesDir)).toBe(templatePath)
    } finally {
      await rm(resourcesDir, { recursive: true, force: true })
    }
  })

  it('resolves the source Lima template from a package workspace in test mode', async () => {
    process.env.NODE_ENV = 'test'
    const workspaceDir = await mkdtemp(join(tmpdir(), 'lima-source-template-'))
    const resourcesDir = join(workspaceDir, 'packages', 'browseros-agent')
    const templatePath = join(
      workspaceDir,
      'packages',
      'build-tools',
      'template',
      'browseros-vm.yaml',
    )
    await mkdir(resourcesDir, { recursive: true })
    await mkdir(dirname(templatePath), { recursive: true })
    await writeFile(templatePath, 'mounts: []\n')

    try {
      expect(resolveBundledLimaTemplate(resourcesDir)).toBe(templatePath)
    } finally {
      await rm(workspaceDir, { recursive: true, force: true })
    }
  })
})

async function createFakeLimactlPath(): Promise<string> {
  const binDir = await mkdtemp(join(tmpdir(), 'host-limactl-'))
  const limactlPath = join(binDir, 'limactl')
  await writeFile(limactlPath, '#!/bin/sh\n')
  await chmod(limactlPath, 0o755)
  process.env.PATH = binDir
  return binDir
}
