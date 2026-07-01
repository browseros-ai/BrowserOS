/**
 * @license
 * Copyright 2025 BrowserOS
 */

import { describe, expect, it, mock } from 'bun:test'
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

interface CapturedCall {
  agent?: string
  command?: string
  cwd?: string
  authPolicy?: string
  timeoutMs?: number
}

interface ProbeResult {
  models: readonly { readonly id: string; readonly name?: string }[]
  configOptions: readonly unknown[]
  reasoning: null
  supportsConfigOption: boolean
  agentInfo: null
  protocolVersion: number
}

let lastCall: CapturedCall | null = null

mock.module('acp-probe', () => ({
  probeAgent: async (input: CapturedCall): Promise<ProbeResult> => {
    lastCall = input
    return {
      models: [{ id: 'gpt-5.4', name: 'GPT-5.4' }],
      configOptions: [],
      reasoning: null,
      supportsConfigOption: false,
      agentInfo: null,
      protocolVersion: 1,
    }
  },
}))

const { probeAcpAgent } = await import(
  '../../../../src/api/services/acpx-probe/probeAgent'
)

describe('probeAcpAgent Codex managed home', () => {
  it('uses auth without copying parser-sensitive config for bundled probes', async () => {
    const tmpRoot = mkdtempSync(join(tmpdir(), 'bos-probe-codex-'))
    const resourcesDir = join(tmpRoot, 'resources')
    const browserosDir = join(tmpRoot, 'browseros')
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
    const previousCodexHome = process.env.CODEX_HOME
    process.env.CODEX_HOME = sourceCodexHome

    try {
      await probeAcpAgent({ agentId: 'codex', browserosDir, resourcesDir })

      const managedCodexHome = join(
        browserosDir,
        'agents',
        'harness',
        'codex-probe',
        'runtime',
        'codex-home',
      )
      expect(lastCall?.agent).toBeUndefined()
      expect(lastCall?.command).toContain(bunPath)
      expect(lastCall?.command).toContain(`CODEX_HOME='${managedCodexHome}'`)
      expect(
        lstatSync(join(managedCodexHome, 'auth.json')).isSymbolicLink(),
      ).toBe(true)
      expect(existsSync(join(managedCodexHome, 'config.toml'))).toBe(false)
    } finally {
      if (previousCodexHome === undefined) {
        delete process.env.CODEX_HOME
      } else {
        process.env.CODEX_HOME = previousCodexHome
      }
      rmSync(tmpRoot, { recursive: true, force: true })
    }
  })
})
