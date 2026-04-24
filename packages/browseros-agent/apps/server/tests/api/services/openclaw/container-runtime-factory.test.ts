/**
 * @license
 * Copyright 2025 BrowserOS
 */

import { afterEach, beforeEach, describe, expect, it } from 'bun:test'
import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import { dirname, join } from 'node:path'
import {
  buildContainerRuntime,
  migrateLegacyOpenClawDir,
} from '../../../../src/api/services/openclaw/container-runtime-factory'
import { logger } from '../../../../src/lib/logger'

describe('container-runtime factory', () => {
  let root: string
  let resourcesDir: string
  let originalNodeEnv: string | undefined

  beforeEach(async () => {
    root = await mkdtemp('/tmp/openclaw-runtime-factory-')
    resourcesDir = join(root, 'resources')
    await mkdir(join(resourcesDir, 'bin', 'third_party', 'lima'), {
      recursive: true,
    })
    await mkdir(join(resourcesDir, 'vm'), { recursive: true })
    await writeFile(
      join(resourcesDir, 'bin', 'third_party', 'lima', 'limactl'),
      '#!/bin/sh\n',
    )
    await writeFile(
      join(resourcesDir, 'vm', 'browseros-vm.yaml'),
      'mounts: []\n',
    )
    originalNodeEnv = process.env.NODE_ENV
    process.env.NODE_ENV = 'production'
  })

  afterEach(async () => {
    if (originalNodeEnv === undefined) {
      delete process.env.NODE_ENV
    } else {
      process.env.NODE_ENV = originalNodeEnv
    }
    await rm(root, { recursive: true, force: true })
  })

  it('rejects non-macOS platforms', () => {
    expect(() =>
      buildContainerRuntime({
        resourcesDir,
        projectDir: join(root, 'project'),
        browserosRoot: root,
        platform: 'linux',
      }),
    ).toThrow('supports macOS only')
  })

  it('returns a disabled runtime on non-macOS platforms in test mode', async () => {
    process.env.NODE_ENV = 'test'

    const runtime = buildContainerRuntime({
      resourcesDir,
      projectDir: join(root, 'project'),
      browserosRoot: root,
      platform: 'linux',
    })

    await expect(runtime.getMachineStatus()).resolves.toEqual({
      initialized: false,
      running: false,
    })
    await expect(runtime.ensureReady()).rejects.toThrow('supports macOS only')
    await expect(runtime.stopVm()).resolves.toBeUndefined()
  })

  it('migrates legacy OpenClaw state into the VM state directory', async () => {
    const legacyFile = join(root, 'openclaw', '.openclaw', 'openclaw.json')
    await mkdir(dirname(legacyFile), { recursive: true })
    await writeFile(legacyFile, '{"ok":true}\n')

    await migrateLegacyOpenClawDir(root)

    await expect(
      readFile(
        join(root, 'vm', 'openclaw', '.openclaw', 'openclaw.json'),
        'utf8',
      ),
    ).resolves.toBe('{"ok":true}\n')
    await expect(readFile(legacyFile, 'utf8')).resolves.toBe('{"ok":true}\n')
  })

  it('leaves both directories in place when new OpenClaw state already exists', async () => {
    const legacyFile = join(root, 'openclaw', 'legacy.txt')
    const newFile = join(root, 'vm', 'openclaw', 'new.txt')
    await mkdir(dirname(legacyFile), { recursive: true })
    await mkdir(dirname(newFile), { recursive: true })
    await writeFile(legacyFile, 'legacy')
    await writeFile(newFile, 'new')
    const originalWarn = logger.warn
    const warnings: string[] = []
    logger.warn = (message) => warnings.push(message)

    try {
      await migrateLegacyOpenClawDir(root)
    } finally {
      logger.warn = originalWarn
    }

    await expect(readFile(legacyFile, 'utf8')).resolves.toBe('legacy')
    await expect(readFile(newFile, 'utf8')).resolves.toBe('new')
    expect(warnings).toContain(
      'OpenClaw legacy and VM state directories both exist',
    )
  })
})
