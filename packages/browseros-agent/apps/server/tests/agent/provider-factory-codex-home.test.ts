/**
 * @license
 * Copyright 2025 BrowserOS
 */

import { afterEach, describe, expect, it, mock } from 'bun:test'
import {
  existsSync,
  lstatSync,
  mkdirSync,
  mkdtempSync,
  rmSync,
  writeFileSync,
} from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import type { ResolvedAgentConfig } from '../../src/agent/types'

let browserosDir = ''
let lastBuildArgs: Record<string, unknown> | null = null
const tempDirs: string[] = []

const fakeProvider = {
  languageModel: () => ({ kind: 'fake-acp-model' }),
  close: async () => {},
  prepare: async () => {},
  setMode: async () => {},
  runtime: { setMode: async () => {} },
}

mock.module('../../src/lib/browseros-dir', () => ({
  getBrowserosDir: () => browserosDir,
}))

mock.module('../../src/lib/agents/acpx-provider/buildAcpxProvider', () => ({
  buildAcpxProvider: async (opts: Record<string, unknown>) => {
    lastBuildArgs = opts
    return fakeProvider
  },
}))

const { createLanguageModel, setEnsureWorkspaceInstructionFileForTesting } =
  await import('../../src/agent/provider-factory')

afterEach(() => {
  setEnsureWorkspaceInstructionFileForTesting(null)
  lastBuildArgs = null
  browserosDir = ''
  for (const dir of tempDirs.splice(0)) {
    rmSync(dir, { recursive: true, force: true })
  }
  delete process.env.CODEX_HOME
})

describe('createLanguageModel Codex managed home', () => {
  it('uses auth without copying parser-sensitive config for bundled Codex chat', async () => {
    const tmpRoot = mkdtempSync(join(tmpdir(), 'bos-provider-codex-'))
    tempDirs.push(tmpRoot)
    const resourcesDir = join(tmpRoot, 'resources')
    browserosDir = join(tmpRoot, 'browseros')
    const sourceCodexHome = join(tmpRoot, 'source-codex-home')
    const binDir = join(resourcesDir, 'bin', 'third_party')
    mkdirSync(binDir, { recursive: true })
    mkdirSync(sourceCodexHome, { recursive: true })
    const bunPath = join(binDir, 'bun')
    writeFileSync(bunPath, '#!/bin/sh\nexit 0\n', { mode: 0o755 })
    writeFileSync(join(sourceCodexHome, 'auth.json'), '{"ok":true}\n')
    writeFileSync(
      join(sourceCodexHome, 'config.toml'),
      '[features.multi_agent_v2]\nenabled = false\n',
    )
    process.env.CODEX_HOME = sourceCodexHome
    setEnsureWorkspaceInstructionFileForTesting(async () => ({
      action: 'skipped-not-new-conversation',
    }))

    const config = {
      conversationId: 'conv-codex-managed-home',
      provider: 'codex',
      model: 'gpt-5.4',
      resourcesDir,
    } satisfies ResolvedAgentConfig
    await createLanguageModel(config)

    const managedCodexHome = join(
      browserosDir,
      'agents',
      'harness',
      'codex-provider',
      'runtime',
      'codex-home',
    )
    const overrides = lastBuildArgs?.agentRegistryOverrides as
      | Record<string, string>
      | undefined
    expect(overrides?.codex).toContain(bunPath)
    expect(overrides?.codex).toContain(`CODEX_HOME='${managedCodexHome}'`)
    expect(
      lstatSync(join(managedCodexHome, 'auth.json')).isSymbolicLink(),
    ).toBe(true)
    expect(existsSync(join(managedCodexHome, 'config.toml'))).toBe(false)
  })
})
