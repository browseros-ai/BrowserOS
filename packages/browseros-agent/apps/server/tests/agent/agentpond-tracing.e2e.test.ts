/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { afterAll, describe, expect, it } from 'bun:test'
import {
  mkdirSync,
  mkdtempSync,
  readdirSync,
  readFileSync,
  rmSync,
} from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

const PROMPT_SENTINEL = 'browseros-agentpond-private-prompt'
const RESPONSE_SENTINEL = 'browseros-agentpond-private-response'
const configuredRoot = process.env.AGENTPOND_E2E_ROOT
const traceRoot =
  configuredRoot ?? mkdtempSync(join(tmpdir(), 'browseros-agentpond-'))

mkdirSync(traceRoot, { recursive: true })

afterAll(() => {
  if (!configuredRoot) rmSync(traceRoot, { force: true, recursive: true })
})

describe('BrowserOS AgentPond tracing', () => {
  it('reads back a content-free trace from the production agent path', async () => {
    const fixture = join(import.meta.dir, 'agentpond-tracing.e2e.fixture.ts')
    const child = Bun.spawn([process.execPath, 'run', fixture], {
      env: {
        ...process.env,
        AGENTPOND_E2E_ROOT: traceRoot,
        AGENTPOND_PROJECT_ID: 'default-project',
        FILES_SDK_PROVIDER: 'fs',
        FILES_SDK_ROOT: traceRoot,
      },
      stderr: 'pipe',
      stdout: 'pipe',
    })
    const [exitCode, stderr] = await Promise.all([
      child.exited,
      new Response(child.stderr).text(),
      new Response(child.stdout).text(),
    ])
    expect(exitCode, stderr).toBe(0)

    const traceKeys = readdirSync(traceRoot, {
      encoding: 'utf8',
      recursive: true,
    }).filter((key) => key.endsWith('.json') && !key.endsWith('.meta.json'))
    expect(traceKeys.length).toBeGreaterThan(0)
    const rawPayload = traceKeys
      .map((key) => readFileSync(join(traceRoot, key), 'utf8'))
      .join('\n')

    expect(rawPayload).toContain('browseros-agentpond-test-model')
    expect(rawPayload).toContain('llm.token_count.total')
    expect(rawPayload).not.toContain(PROMPT_SENTINEL)
    expect(rawPayload).not.toContain(RESPONSE_SENTINEL)
  })
})
