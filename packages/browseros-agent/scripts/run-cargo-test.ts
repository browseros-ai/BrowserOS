#!/usr/bin/env bun
/**
 * Runs `cargo` (typically `test --workspace --locked`), streams its output live,
 * and parses libtest's "test result:" summary lines into an aggregate JUnit at
 * BROWSEROS_JUNIT_PATH so the PR test summary reports real counts instead of
 * 0/0. cargo test emits no JUnit natively; parsing its output keeps doctests
 * running (unlike cargo-nextest). The process exits with cargo's exit code, so
 * this never changes whether the job passes: a parser miss only affects counts.
 *
 *   bun run ./scripts/run-cargo-test.ts test --workspace --locked
 */

import { spawn } from 'node:child_process'
import { mkdirSync, writeFileSync } from 'node:fs'
import { basename, dirname, resolve } from 'node:path'
import {
  buildCargoJunitXml,
  parseCargoTestCounts,
  reconcileCargoCounts,
} from './run-cargo-test.helpers'

const args = process.argv.slice(2)
const junitPath = process.env.BROWSEROS_JUNIT_PATH?.trim()

const child = spawn('cargo', args, { stdio: ['inherit', 'pipe', 'pipe'] })

let combined = ''
child.stdout.on('data', (chunk) => {
  const text = chunk.toString()
  combined += text
  process.stdout.write(text)
})
child.stderr.on('data', (chunk) => {
  const text = chunk.toString()
  combined += text
  process.stderr.write(text)
})

const exitCode: number = await new Promise((res) => {
  child.on('error', () => res(1))
  child.on('close', (code) => res(code ?? 1))
})

if (junitPath) {
  const suite = basename(junitPath).replace(/\.xml$/, '') || 'cargo'
  const counts = reconcileCargoCounts(parseCargoTestCounts(combined), exitCode)
  const outputPath = resolve(junitPath)
  mkdirSync(dirname(outputPath), { recursive: true })
  writeFileSync(outputPath, buildCargoJunitXml(suite, counts))
}

process.exit(exitCode)
