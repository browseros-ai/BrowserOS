/**
 * @license
 * Copyright 2025 BrowserOS
 */

import { afterEach, describe, expect, it } from 'bun:test'
import { mkdtemp, readdir, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import {
  deriveRuntimeSessionKey,
  loadLatestRuntimeState,
  saveLatestRuntimeState,
} from '../../../src/lib/agents/acpx/runtime-state'

describe('acpx runtime state', () => {
  const tempDirs: string[] = []

  afterEach(async () => {
    await Promise.all(
      tempDirs.map((dir) => rm(dir, { recursive: true, force: true })),
    )
    tempDirs.length = 0
  })

  it('saves and loads latest runtime state atomically', async () => {
    const dir = await mkdtemp(join(tmpdir(), 'browseros-runtime-state-'))
    tempDirs.push(dir)
    const filePath = join(dir, 'agent-1.json')

    await saveLatestRuntimeState(filePath, {
      sessionId: 'main',
      runtimeSessionKey: 'agent:agent-1:main:abc',
      cwd: '/tmp/work',
      agentHome: '/tmp/agent-home',
      updatedAt: 1234,
    })

    expect(await loadLatestRuntimeState(filePath)).toEqual({
      sessionId: 'main',
      runtimeSessionKey: 'agent:agent-1:main:abc',
      cwd: '/tmp/work',
      agentHome: '/tmp/agent-home',
      updatedAt: 1234,
    })
    expect(
      (await readdir(dir)).filter((name) => name.includes('.tmp')),
    ).toEqual([])
  })

  it('returns null when runtime state is absent or malformed', async () => {
    const dir = await mkdtemp(join(tmpdir(), 'browseros-runtime-state-'))
    tempDirs.push(dir)

    expect(await loadLatestRuntimeState(join(dir, 'missing.json'))).toBeNull()
  })

  it('derives stable session keys and changes when identity inputs change', () => {
    const base = {
      agentId: 'agent-1',
      sessionId: 'main' as const,
      adapter: 'codex',
      cwd: '/tmp/work',
      agentHome: '/tmp/agent-home',
      promptVersion: 'v1',
      skillIdentity: 'skills-v1',
      commandIdentity: 'codex-home-v1',
    }

    const first = deriveRuntimeSessionKey(base)
    expect(first).toMatch(/^agent:agent-1:main:[a-f0-9]{16}$/)
    expect(deriveRuntimeSessionKey(base)).toBe(first)
    expect(
      deriveRuntimeSessionKey({ ...base, cwd: '/tmp/other-work' }),
    ).not.toBe(first)
    expect(
      deriveRuntimeSessionKey({ ...base, skillIdentity: 'skills-v2' }),
    ).not.toBe(first)
  })

  it('derives stable session keys for UUID sessions', () => {
    const sessionId = '00000000-0000-4000-8000-000000000001'
    const base = {
      agentId: 'agent-1',
      sessionId,
      adapter: 'codex',
      cwd: '/tmp/work',
      agentHome: '/tmp/agent-home',
      promptVersion: 'v1',
      skillIdentity: 'skills-v1',
      commandIdentity: 'codex-home-v1',
    }

    const first = deriveRuntimeSessionKey(base)

    expect(first).toMatch(
      /^agent:agent-1:00000000-0000-4000-8000-000000000001:[a-f0-9]{16}$/,
    )
    expect(deriveRuntimeSessionKey(base)).toBe(first)
    expect(deriveRuntimeSessionKey({ ...base, sessionId: 'main' })).not.toBe(
      first,
    )
  })

  it('saves and loads latest runtime state for a UUID session', async () => {
    const dir = await mkdtemp(join(tmpdir(), 'browseros-runtime-state-'))
    tempDirs.push(dir)
    const filePath = join(
      dir,
      'agent-1',
      '00000000-0000-4000-8000-000000000001.json',
    )

    await saveLatestRuntimeState(filePath, {
      sessionId: '00000000-0000-4000-8000-000000000001',
      runtimeSessionKey:
        'agent:agent-1:00000000-0000-4000-8000-000000000001:abc',
      cwd: '/tmp/work',
      agentHome: '/tmp/agent-home',
      updatedAt: 1234,
    })

    expect(await loadLatestRuntimeState(filePath)).toEqual({
      sessionId: '00000000-0000-4000-8000-000000000001',
      runtimeSessionKey:
        'agent:agent-1:00000000-0000-4000-8000-000000000001:abc',
      cwd: '/tmp/work',
      agentHome: '/tmp/agent-home',
      updatedAt: 1234,
    })
  })
})
