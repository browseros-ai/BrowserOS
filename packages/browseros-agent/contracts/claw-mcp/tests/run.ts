#!/usr/bin/env bun
/**
 * Entry point for the claw-mcp contract suite:
 *   bun contracts/claw-mcp/tests/run.ts [--smoke]
 *
 * Skips instantly (exit 0) when BROWSEROS_BINARY is unset so it is
 * safe inside `bun run test` anywhere; otherwise pre-builds the rust
 * server (outside test timeouts) and execs the Rust conformance suite.
 */

import { resolve } from 'node:path'
import { buildRustServer } from './rust-server'

const smoke = process.argv.includes('--smoke')

if (!process.env.BROWSEROS_BINARY) {
  console.log(
    'claw-mcp contract suite skipped: BROWSEROS_BINARY is not set (point it at a BrowserOS/BrowserClaw executable to run)',
  )
  process.exit(0)
}

buildRustServer()

const result = Bun.spawnSync({
  cmd: ['bun', 'test', resolve(import.meta.dir, 'rust-conformance.test.ts')],
  env: {
    ...process.env,
    ...(smoke ? { CLAW_MCP_SMOKE: '1' } : {}),
  },
  stdout: 'inherit',
  stderr: 'inherit',
})
process.exit(result.exitCode ?? 1)
