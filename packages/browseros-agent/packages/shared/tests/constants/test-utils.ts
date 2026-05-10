/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Shared test utilities for constant env-override tests.
 */

/**
 * Run a one-liner in a child Bun process with custom env vars.
 * Guarantees a fresh module cache so process.env is read at import time
 * with the overridden values — no state leakage between tests.
 */
export async function spawnWithEnv(
  envOverrides: Record<string, string>,
  code: string,
): Promise<string> {
  const proc = Bun.spawn(['bun', '-e', code], {
    env: { ...process.env, ...envOverrides, NO_COLOR: '1', FORCE_COLOR: '0' },
    stdout: 'pipe',
    stderr: 'pipe',
  })
  await proc.exited
  const stdout = await new Response(proc.stdout).text()
  if (proc.exitCode !== 0) {
    const stderr = await new Response(proc.stderr).text()
    throw new Error(`Child process exited ${proc.exitCode}: ${stderr}\n${stdout}`)
  }
  return stdout.trim()
}
