import { expect, test } from 'bun:test'
import { existsSync, mkdtempSync, readFileSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { start } from './environment.mjs'

// start() takes four consecutive ports from its base (both browsers' CDP, the Neo server, the
// fixture server). check.sh runs this suite, so the CLI's fixed base made two check.sh runs in
// different worktrees collide here. Port 0 cannot pick a block: macOS hands out ephemeral ports
// from one counter in sequence, so concurrent runs got neighbouring bases and overlapping blocks
// again and again. A random base below every OS's ephemeral range (Linux 32768+, macOS 49152+)
// keeps concurrent runs apart. start() probes all four ports before it launches anything, so a
// taken one rejects with EADDRINUSE before any child or processes.json exists, and the test draws
// another base.
async function startOnFreePorts(options) {
  for (let attempt = 1; ; attempt++) {
    const ports = 20000 + Math.floor(Math.random() * 12000)
    try {
      return await start({ ...options, ports })
    } catch (error) {
      if (error?.code !== 'EADDRINUSE' || attempt === 10) throw error
    }
  }
}

test('failed browser startup rejects promptly and removes only its own temporary profiles', async () => {
  const output = mkdtempSync(join(tmpdir(), 'neo-env-test-'))
  try {
    await expect(
      startOnFreePorts({
        browser: join(output, 'missing-browser'),
        neoBinary: join(output, 'missing-server'),
        assetsDir: output,
        runDir: output,
        gitSha: 'test',
      }),
    ).rejects.toThrow()
    const children = JSON.parse(readFileSync(join(output, 'processes.json')))
    for (const child of children)
      for (const arg of child.spawnargs)
        if (arg.startsWith('--user-data-dir=')) {
          const profile = arg.slice('--user-data-dir='.length)
          expect(existsSync(profile)).toBe(false)
        }
  } finally {
    rmSync(output, { recursive: true, force: true })
  }
}, 5000)
